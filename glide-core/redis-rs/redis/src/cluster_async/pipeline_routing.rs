use crate::aio::ConnectionLike;
use crate::cluster_async::ClusterConnInner;
use crate::cluster_async::Connect;

use crate::cluster_routing::{
    command_for_multi_slot_indices, MultipleNodeRoutingInfo, ResponsePolicy, SingleNodeRoutingInfo,
};
use crate::{cluster_routing, RedisResult, Value};
use crate::{cluster_routing::Route, Cmd, ErrorKind, RedisError};
use std::collections::HashMap;
use std::sync::Arc;

use crate::cluster_async::MUTEX_READ_ERR;
use crate::Pipeline;
use futures::FutureExt;
use rand::prelude::IteratorRandom;
use tokio::sync::oneshot;
use tokio::sync::oneshot::error::RecvError;

use super::CmdArg;
use super::PendingRequest;
use super::RedirectNode;
use super::RequestInfo;
use super::{Core, InternalSingleNodeRouting, OperationTarget, Response};

/// Represents a pipeline command execution context for a specific node
#[derive(Default, Debug)]
pub struct NodePipelineContext<C> {
    /// The pipeline of commands to be executed
    pub pipeline: Pipeline,
    /// The connection to the node
    pub connection: C,
    /// Vector of (command_index, inner_index) pairs tracking command order
    /// command_index: Position in the original pipeline
    /// inner_index: Optional sub-index for multi-node operations (e.g. MSET)
    pub command_indices: Vec<(usize, Option<usize>)>,
}

/// Maps node addresses to their pipeline execution contexts
pub type NodePipelineMap<C> = HashMap<String, NodePipelineContext<C>>;

impl<C> NodePipelineContext<C> {
    fn new(connection: C) -> Self {
        Self {
            pipeline: Pipeline::new(),
            connection,
            command_indices: Vec::new(),
        }
    }

    // Adds a command to the pipeline and records its index
    fn add_command(&mut self, cmd: Cmd, index: usize, inner_index: Option<usize>) {
        self.pipeline.add_command(cmd);
        self.command_indices.push((index, inner_index));
    }
}

/// `NodeResponse` represents a response from a node along with its source node address.
type NodeResponse = (Value, String);
/// `PipelineResponses` represents the responses for each pipeline command.
/// The outer `Vec` represents the pipeline commands, and each inner `Vec` contains (response, address) pairs.
/// Since some commands can be executed across multiple nodes (e.g., multi-node commands), a single command
/// might produce multiple responses, each from a different node. By storing the responses with their
/// respective node addresses, we ensure that we have all the information needed to aggregate the results later.
pub type PipelineResponses = Vec<Vec<NodeResponse>>;

/// `AddressAndIndices` represents the address of a node and the indices of commands associated with that node.
type AddressAndIndices = Vec<(String, Vec<(usize, Option<usize>)>)>;

/// Adds a command to the pipeline map for a specific node address.
pub fn add_command_to_node_pipeline_map<C>(
    pipeline_map: &mut NodePipelineMap<C>,
    address: String,
    connection: C,
    cmd: Cmd,
    index: usize,
    inner_index: Option<usize>,
) {
    pipeline_map
        .entry(address)
        .or_insert_with(|| NodePipelineContext::new(connection))
        .add_command(cmd, index, inner_index);
}

/// Adds a command to a random existing node pipeline in the pipeline map
pub fn add_command_to_random_existing_node<C>(
    pipeline_map: &mut NodePipelineMap<C>,
    cmd: Cmd,
    index: usize,
) -> RedisResult<()> {
    let mut rng = rand::thread_rng();
    if let Some(node_context) = pipeline_map.values_mut().choose(&mut rng) {
        node_context.add_command(cmd, index, None);
        Ok(())
    } else {
        Err(RedisError::from((ErrorKind::IoError, "No nodes available")))
    }
}

/// Maps the commands in a pipeline to the appropriate nodes based on their routing information.
///
/// This function processes each command in the given pipeline, determines its routing information,
/// and organizes it into a map of node pipelines. It handles both single-node and multi-node routing
/// strategies and ensures that the commands are distributed accordingly.
///
/// It also collects response policies for multi-node routing and returns them along with the pipeline map.
/// This is to ensure we can aggregate responses from properly from the different nodes.
///
/// # Arguments
///
/// * `pipeline` - A reference to the pipeline containing the commands to route.
/// * `core` - The core object that provides access to connection locks and other resources.
///
/// # Returns
///
/// A `RedisResult` containing a tuple:
///
/// - A `NodePipelineMap<C>` where commands are grouped by their corresponding nodes (as pipelines).
/// - A `Vec<(usize, MultipleNodeRoutingInfo, Option<ResponsePolicy>)>` containing the routing information
///   and response policies for multi-node commands, along with the index of the command in the pipeline, for aggregating the responses later.
pub async fn map_pipeline_to_nodes<C>(
    pipeline: &crate::Pipeline,
    core: Core<C>,
) -> RedisResult<(
    NodePipelineMap<C>,
    Vec<(usize, MultipleNodeRoutingInfo, Option<ResponsePolicy>)>,
)>
where
    C: Clone + ConnectionLike + Connect + Send + Sync + 'static,
{
    let mut pipelines_by_connection = NodePipelineMap::new();
    let mut response_policies = Vec::new();

    for (index, cmd) in pipeline.cmd_iter().enumerate() {
        match cluster_routing::RoutingInfo::for_routable(cmd).unwrap_or(
            cluster_routing::RoutingInfo::SingleNode(SingleNodeRoutingInfo::Random),
        ) {
            cluster_routing::RoutingInfo::SingleNode(route) => {
                handle_pipeline_single_node_routing(
                    &mut pipelines_by_connection,
                    cmd.clone(),
                    route.into(),
                    core.clone(),
                    index,
                )
                .await
                .map_err(|(_target, err)| err)?; // todo - handle error
            }

            cluster_routing::RoutingInfo::MultiNode((multi_node_routing, response_policy)) => {
                //save the routing info and response policy, so we will be able to aggregate the results later
                response_policies.push((index, multi_node_routing.clone(), response_policy));
                match multi_node_routing {
                    MultipleNodeRoutingInfo::AllNodes | MultipleNodeRoutingInfo::AllMasters => {
                        let connections: Vec<_> = {
                            let lock = core.conn_lock.read().expect(MUTEX_READ_ERR);
                            if matches!(multi_node_routing, MultipleNodeRoutingInfo::AllNodes) {
                                lock.all_node_connections().collect()
                            } else {
                                lock.all_primary_connections().collect()
                            }
                        };
                        for (inner_index, (address, conn)) in connections.into_iter().enumerate() {
                            add_command_to_node_pipeline_map(
                                &mut pipelines_by_connection,
                                address,
                                conn.await,
                                cmd.clone(),
                                index,
                                Some(inner_index),
                            );
                        }
                    }
                    MultipleNodeRoutingInfo::MultiSlot((slots, _)) => {
                        handle_pipeline_multi_slot_routing(
                            &mut pipelines_by_connection,
                            core.clone(),
                            cmd,
                            index,
                            slots,
                        )
                        .await;
                    }
                }
            }
        }
    }
    Ok((pipelines_by_connection, response_policies))
}

/// Handles pipeline commands that require single-node routing.
///
/// This function processes commands with `SingleNode` routing information and determines
/// the appropriate handling based on the routing type.
///
/// ### Parameters:
/// - `pipeline_map`: A mutable reference to the `NodePipelineMap`, representing the pipelines grouped by nodes.
/// - `cmd`: The command to process and add to the appropriate node pipeline.
/// - `routing`: The single-node routing information, which determines how the command is routed.
/// - `core`: The core object responsible for managing connections and routing logic.
/// - `index`: The position of the command in the overall pipeline.
pub async fn handle_pipeline_single_node_routing<C>(
    pipeline_map: &mut NodePipelineMap<C>,
    cmd: Cmd,
    routing: InternalSingleNodeRouting<C>,
    core: Core<C>,
    index: usize,
) -> Result<(), (OperationTarget, RedisError)>
where
    C: Clone + ConnectionLike + Connect + Send + Sync + 'static,
{
    if matches!(routing, InternalSingleNodeRouting::Random) && !pipeline_map.is_empty() {
        // The routing info is to a random node, and we already have sub-pipelines within our pipelines map, so just add it to a random sub-pipeline
        add_command_to_random_existing_node(pipeline_map, cmd, index)
            .map_err(|err| (OperationTarget::NotFound, err))?;
        Ok(())
    } else {
        let (address, conn) =
            ClusterConnInner::get_connection(routing, core, Some(Arc::new(cmd.clone())))
                .await
                .map_err(|err| (OperationTarget::NotFound, err))?;
        add_command_to_node_pipeline_map(pipeline_map, address, conn, cmd, index, None);
        Ok(())
    }
}

/// Handles multi-slot commands within a pipeline.
///
/// This function processes commands with routing information indicating multiple slots
/// (e.g., `MSET` or `MGET`), splits them into sub-commands based on their target slots,
/// and assigns these sub-commands to the appropriate pipelines for the corresponding nodes.
///
/// ### Parameters:
/// - `pipelines_by_connection`: A mutable map of node pipelines where the commands will be added.
/// - `core`: The core structure that provides access to connection management.
/// - `cmd`: The original multi-slot command that needs to be split.
/// - `index`: The index of the original command within the pipeline.
/// - `slots`: A vector containing routing information. Each entry includes:
///   - `Route`: The specific route for the slot.
///   - `Vec<usize>`: Indices of the keys within the command that map to this slot.
pub async fn handle_pipeline_multi_slot_routing<C>(
    pipelines_by_connection: &mut NodePipelineMap<C>,
    core: Core<C>,
    cmd: &Cmd,
    index: usize,
    slots: Vec<(Route, Vec<usize>)>,
) where
    C: Clone,
{
    // inner_index is used to keep track of the index of the sub-command inside cmd
    for (inner_index, (route, indices)) in slots.iter().enumerate() {
        let conn = {
            let lock = core.conn_lock.read().expect(MUTEX_READ_ERR);
            lock.connection_for_route(route)
        };
        if let Some((address, conn)) = conn {
            // create the sub-command for the slot
            let new_cmd = command_for_multi_slot_indices(cmd, indices.iter());
            add_command_to_node_pipeline_map(
                pipelines_by_connection,
                address,
                conn.await,
                new_cmd,
                index,
                Some(inner_index),
            );
        }
    }
}

/// Creates `PendingRequest` objects for each pipeline in the provided pipeline map.
///
/// This function processes the given map of node pipelines and prepares each sub-pipeline for execution
/// by creating a `PendingRequest` containing all necessary details for execution.
/// Additionally, it sets up communication channels to asynchronously receive the results of each sub-pipeline's execution.
///
/// Returns a tuple containing:
/// - **receivers**: A vector of `oneshot::Receiver` objects to receive the responses of the sub-pipeline executions.
/// - **pending_requests**: A vector of `PendingRequest` objects, each representing a pipeline scheduled for execution on a node.
/// - **addresses_and_indices**: A vector of tuples containing node addresses and their associated command indices for each sub-pipeline,
///   allowing the results to be mapped back to their original command within the original pipeline.
#[allow(clippy::type_complexity)]
pub fn collect_pipeline_requests<C>(
    pipelines_by_connection: NodePipelineMap<C>,
) -> (
    Vec<oneshot::Receiver<RedisResult<Response>>>,
    Vec<PendingRequest<C>>,
    Vec<(String, Vec<(usize, Option<usize>)>)>,
)
where
    C: Clone + ConnectionLike + Connect + Send + Sync + 'static,
{
    let mut receivers = Vec::new();
    let mut pending_requests = Vec::new();
    let mut addresses_and_indices = Vec::new();

    for (address, context) in pipelines_by_connection {
        println!("Address is {address} Context is: {:?}", context.pipeline);
        // Create a channel to receive the pipeline execution results
        let (sender, receiver) = oneshot::channel();
        // Add the receiver to the list of receivers
        receivers.push(receiver);
        pending_requests.push(PendingRequest {
            retry: 0,
            sender,
            info: RequestInfo {
                cmd: CmdArg::Pipeline {
                    count: context.pipeline.len(),
                    pipeline: context.pipeline.into(),
                    offset: 0,
                    route: InternalSingleNodeRouting::Connection {
                        address: address.clone(),
                        conn: async { context.connection }.boxed().shared(),
                    },
                    // mark it as a sub-pipeline mode
                    sub_pipeline: true,
                },
            },
        });
        // Record the node address and its associated command indices for result mapping
        addresses_and_indices.push((address, context.command_indices));
    }

    (receivers, pending_requests, addresses_and_indices)
}

/// Adds the result of a pipeline command to the `pipeline_responses` collection.
///
/// This function updates the `pipeline_responses` vector at the given `index` and optionally at the
/// `inner_index` if provided. If `inner_index` is `Some`, it ensures that the vector at that index is large enough
/// to hold the value and address at the specified position, resizing it if necessary. If `inner_index` is `None`,
/// the value and address are simply appended to the vector.
///
/// # Parameters
/// - `pipeline_responses`: A mutable reference to a vector of vectors that stores the results of pipeline commands.
/// - `index`: The index in `pipeline_responses` where the result should be stored.
/// - `inner_index`: An optional index within the vector at `index`, used to store the result at a specific position.
/// - `value`: The result value to store.
/// - `address`: The address associated with the result.
pub fn add_pipeline_result(
    pipeline_responses: &mut PipelineResponses,
    index: usize,
    inner_index: Option<usize>,
    value: Value,
    address: String,
) {
    match inner_index {
        Some(inner_index) => {
            // Ensure the vector at the given index is large enough to hold the value and address at the specified position
            if pipeline_responses[index].len() <= inner_index {
                pipeline_responses[index].resize(inner_index + 1, (Value::Nil, "".to_string()));
            }
            pipeline_responses[index][inner_index] = (value, address);
        }
        None => pipeline_responses[index].push((value, address)),
    }
}

/// Processes the responses of pipeline commands and updates the given `pipeline_responses`
/// with the corresponding results.
///
/// The function iterates over the responses along with the `addresses_and_indices` list,
/// ensuring that each response is added to its appropriate position in `pipeline_responses` along with the associated address.
/// If any response indicates an error, the function terminates early and returns the first encountered error.
///
/// # Parameters
///
/// - `pipeline_responses`: A vec that holds the original pipeline commands responses.
/// - `responses`: A list of responses corresponding to each sub-pipeline.
/// - `addresses_and_indices`: A list of (address, indices) pairs indicating where each response should be placed.
///
/// # Returns
///
/// - `Ok(())` if all responses are processed successfully.
/// - `Err((OperationTarget, RedisError))` if a node-level or reception error occurs.
pub async fn process_pipeline_responses<C>(
    pipeline_responses: &mut PipelineResponses,
    responses: Vec<Result<RedisResult<Response>, RecvError>>,
    addresses_and_indices: AddressAndIndices,
    pipeline: &crate::Pipeline,
    core: Core<C>,
) -> Result<((Vec<(usize, Option<usize>)>, Vec<RedisError>)), (OperationTarget, RedisError)>
where
    C: Clone + ConnectionLike + Connect + Send + Sync + 'static,
{
    let mut moved_error_indices = Vec::new();
    let mut errors = Vec::new();

    for ((address, command_indices), response_result) in
        addresses_and_indices.into_iter().zip(responses)
    {
        match response_result {
            Ok(Ok(Response::Multiple(values))) => {
                // Add each response to the pipeline_responses vector at the appropriate index
                for ((index, inner_index), value) in command_indices.into_iter().zip(values) {
                    match value.clone() {
                        Value::ServerError(e)
                            if matches!(e.kind(), ErrorKind::Moved | ErrorKind::Ask) =>
                        {
                            println!("MOVED/ASK error, index: {}", index);
                            if matches!(e.kind(), ErrorKind::Moved) {
                                println!("MOVED error");
                                // update upon moved
                                let r: RedisError = e.clone().into();
                                let x = RedirectNode::from_option_tuple(r.redirect_node()).unwrap();
                                let address = x.address.clone();

                                ClusterConnInner::update_upon_moved_error(
                                    core.clone(),
                                    x.slot,
                                    x.address.into(),
                                )
                                .await
                                .map_err(|err| (address.into(), err))?;
                            }

                            moved_error_indices.push((index, inner_index));
                            errors.push(e.into());
                        }
                        _ => {
                            add_pipeline_result(
                                pipeline_responses,
                                index,
                                inner_index,
                                value,
                                address.clone(),
                            );
                        }
                    }
                }
            }
            Ok(Err(err)) => {
                println!("Error is: {:?}", err);
                // apply to all commands in the pipeline
                return Err((OperationTarget::Node { address }, err));
            }
            _ => {
                return Err((
                    OperationTarget::Node { address },
                    RedisError::from((ErrorKind::ResponseError, "Failed to receive response")),
                ));
            }
        }
    }

    Ok((moved_error_indices, errors))
}

/// Handles commands that encountered a MOVED error during pipeline execution.
///
/// This function processes commands that resulted in a MOVED error, indicating that the key
/// has been moved to a different node. It creates a new pipeline from the commands that encountered
/// the MOVED error and retries their execution.
///
/// # Arguments
///
/// * `pipeline_responses` - A mutable reference to the collection of pipeline responses.
/// * `pipeline` - A reference to the original pipeline containing the commands.
/// * `moved_error_indices` - A vector of indices indicating which commands encountered a MOVED error.
/// * `core` - The core object that provides access to connection locks and other resources.
///
/// # Returns
///
/// A `Result` indicating the success or failure of handling the MOVED commands.
pub async fn handle_moved_commands<C>(
    pipeline_responses: &mut PipelineResponses,
    pipeline: &crate::Pipeline,
    moved_error_indices: Vec<(usize, Option<usize>)>,
    errors: Vec<RedisError>,
    core: Core<C>,
) -> Result<
    (
        Vec<Result<RedisResult<Response>, RecvError>>,
        Vec<(String, Vec<(usize, Option<usize>)>)>,
    ),
    (OperationTarget, RedisError),
>
where
    C: Clone + ConnectionLike + Connect + Send + Sync + 'static,
{
    println!("Handling moved commands");
    // Create a pipeline from moved_error_indices
    let mut pipeline_map = NodePipelineMap::new();
    for ((index, inner_index), error) in moved_error_indices.clone().into_iter().zip(errors) {
        if let Some(cmd) = pipeline.get_command(index) {
            println!("# Command is: {:?}", cmd);
            let redirect = InternalSingleNodeRouting::Redirect {
                redirect: error.redirect(),
                previous_routing: Box::new(InternalSingleNodeRouting::Random::<C>),
            };
            let (address, conn) = ClusterConnInner::get_connection(
                redirect,
                core.clone(),
                Some(Arc::new(cmd.clone())),
            )
            .await
            .map_err(|err| (OperationTarget::NotFound, err))?;

            println!("# Address is: {}", address);
            add_command_to_node_pipeline_map(
                &mut pipeline_map,
                address,
                conn,
                cmd.clone(),
                index,
                inner_index,
            );
        }
    }

    let (receivers, pending_requests, addresses_and_indices) =
        collect_pipeline_requests(pipeline_map);

    // Add the pending requests to the pending_requests queue
    core.pending_requests
        .lock()
        .unwrap()
        .extend(pending_requests.into_iter());

    // Wait for all receivers to complete and collect the responses
    let responses: Vec<_> = futures::future::join_all(receivers.into_iter())
        .await
        .into_iter()
        .collect();

    Ok((responses, addresses_and_indices))
}

/// Processes the pipeline responses and handles any MOVED errors by retrying the commands.
///
/// This function serves as a loop that processes the pipeline responses and handles any MOVED errors
/// by retrying the commands that encountered the error. It continues to process and retry until all
/// commands are successfully executed or an unrecoverable error occurs.
///
/// # Arguments
///
/// * `pipeline_responses` - A mutable reference to the collection of pipeline responses.
/// * `responses` - A list of responses corresponding to each sub-pipeline.
/// * `addresses_and_indices` - A list of (address, indices) pairs indicating where each response should be placed.
/// * `pipeline` - A reference to the original pipeline containing the commands.
/// * `core` - The core object that provides access to connection locks and other resources.
///
/// # Returns
///
/// A `Result` indicating the success or failure of processing the pipeline responses.
pub async fn process_and_retry_pipeline_responses<C>(
    pipeline_responses: &mut PipelineResponses,
    mut responses: Vec<Result<RedisResult<Response>, RecvError>>,
    mut addresses_and_indices: AddressAndIndices,
    pipeline: &crate::Pipeline,
    core: Core<C>,
) -> Result<(), (OperationTarget, RedisError)>
where
    C: Clone + ConnectionLike + Connect + Send + Sync + 'static,
{
    let mut retries = 10;
    loop {
        match process_pipeline_responses(
            pipeline_responses,
            responses,
            addresses_and_indices,
            pipeline,
            core.clone(),
        )
        .await
        {
            Ok((moved_error_indices, errors)) => {
                if moved_error_indices.is_empty() {
                    break Ok(());
                }
                (responses, addresses_and_indices) = handle_moved_commands(
                    pipeline_responses,
                    pipeline,
                    moved_error_indices,
                    errors,
                    core.clone(),
                )
                .await?;
            }

            Err(e) => break Err(e),
        }
        retries -= 1;
        if retries == 0 {
            break Err((
                OperationTarget::NotFound,
                RedisError::from((ErrorKind::ResponseError, "Too many retries")),
            ));
        }
    }
}

/// This function returns the route for a given pipeline.
/// The function goes over the commands in the pipeline, checks that all key-based commands are routed to the same slot,
/// and returns the route for that specific node.
/// If the pipeline contains no key-based commands, the function returns None.
/// For non-atomic pipelines, the function will return None, regardless of the commands in it.
pub fn route_for_pipeline(pipeline: &crate::Pipeline) -> RedisResult<Option<Route>> {
    fn route_for_command(cmd: &Cmd) -> Option<Route> {
        match cluster_routing::RoutingInfo::for_routable(cmd) {
            Some(cluster_routing::RoutingInfo::SingleNode(SingleNodeRoutingInfo::Random)) => None,
            Some(cluster_routing::RoutingInfo::SingleNode(
                SingleNodeRoutingInfo::SpecificNode(route),
            )) => Some(route),
            Some(cluster_routing::RoutingInfo::SingleNode(
                SingleNodeRoutingInfo::RandomPrimary,
            )) => Some(Route::new_random_primary()),
            Some(cluster_routing::RoutingInfo::MultiNode(_)) => None,
            Some(cluster_routing::RoutingInfo::SingleNode(SingleNodeRoutingInfo::ByAddress {
                ..
            })) => None,
            None => None,
        }
    }

    if pipeline.is_atomic() {
        // Find first specific slot and send to it. There's no need to check If later commands
        // should be routed to a different slot, since the server will return an error indicating this.
        pipeline
            .cmd_iter()
            .map(route_for_command)
            .try_fold(None, |chosen_route, next_cmd_route| {
                match (chosen_route, next_cmd_route) {
                    (None, _) => Ok(next_cmd_route),
                    (_, None) => Ok(chosen_route),
                    (Some(chosen_route), Some(next_cmd_route)) => {
                        if chosen_route.slot() != next_cmd_route.slot() {
                            Err((
                                ErrorKind::CrossSlot,
                                "Received crossed slots in transaction",
                            )
                                .into())
                        } else {
                            Ok(Some(chosen_route))
                        }
                    }
                }
            })
    } else {
        // Pipeline is not atomic, so we can have commands with different slots.
        Ok(None)
    }
}
