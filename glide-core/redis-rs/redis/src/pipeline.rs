#![macro_use]

use crate::cmd::{cmd, cmd_len, cmd_len_arc, Cmd};
use crate::connection::ConnectionLike;
use crate::types::{
    from_owned_redis_value, ErrorKind, FromRedisValue, HashSet, RedisResult, ToRedisArgs, Value,
};
use std::sync::Arc;

/// Represents a redis command pipeline.
#[derive(Clone)]
pub struct Pipeline {
    // Now storing each command as an Arc
    commands: Vec<Arc<Cmd>>,
    transaction_mode: bool,
    ignored_commands: HashSet<usize>,
}

impl Pipeline {
    /// Creates an empty pipeline. For consistency with the `cmd`
    /// API a `pipe` function is provided as an alias.
    pub fn new() -> Pipeline {
        Self::with_capacity(0)
    }

    /// Creates an empty pipeline with pre-allocated capacity.
    pub fn with_capacity(capacity: usize) -> Pipeline {
        Pipeline {
            commands: Vec::with_capacity(capacity),
            transaction_mode: false,
            ignored_commands: HashSet::new(),
        }
    }

    /// Enables atomic mode. In atomic mode the whole pipeline is
    /// enclosed in `MULTI`/`EXEC`.
    #[inline]
    pub fn atomic(&mut self) -> &mut Pipeline {
        self.transaction_mode = true;
        self
    }

    /// Returns the encoded pipeline commands.
    pub fn get_packed_pipeline(&self) -> Vec<u8> {
        encode_pipeline(&self.commands, self.transaction_mode)
    }

    #[cfg(feature = "aio")]
    pub(crate) fn write_packed_pipeline(&self, out: &mut Vec<u8>) {
        write_pipeline(out, &self.commands, self.transaction_mode)
    }

    fn execute_pipelined(&self, con: &mut dyn ConnectionLike) -> RedisResult<Value> {
        Ok(self.make_pipeline_results(con.req_packed_commands(
            &encode_pipeline(&self.commands, false),
            0,
            self.commands.len(),
        )?))
    }

    fn execute_transaction(&self, con: &mut dyn ConnectionLike) -> RedisResult<Value> {
        let mut resp = con.req_packed_commands(
            &encode_pipeline(&self.commands, true),
            self.commands.len() + 1,
            1,
        )?;
        match resp.pop() {
            Some(Value::Nil) => Ok(Value::Nil),
            Some(Value::Array(items)) => Ok(self.make_pipeline_results(items)),
            _ => fail!((
                ErrorKind::ResponseError,
                "Invalid response when parsing multi response"
            )),
        }
    }

    /// Executes the pipeline and fetches the return values.
    #[inline]
    pub fn query<T: FromRedisValue>(&self, con: &mut dyn ConnectionLike) -> RedisResult<T> {
        if !con.supports_pipelining() {
            fail!((
                ErrorKind::ResponseError,
                "This connection does not support pipelining."
            ));
        }
        from_owned_redis_value(if self.commands.is_empty() {
            Value::Array(vec![])
        } else if self.transaction_mode {
            self.execute_transaction(con)?
        } else {
            self.execute_pipelined(con)?
        })
    }

    #[cfg(feature = "aio")]
    async fn execute_pipelined_async<C>(&self, con: &mut C) -> RedisResult<Value>
    where
        C: crate::aio::ConnectionLike,
    {
        let value = con
            .req_packed_commands(self, 0, self.commands.len())
            .await?;
        Ok(self.make_pipeline_results(value))
    }

    #[cfg(feature = "aio")]
    async fn execute_transaction_async<C>(&self, con: &mut C) -> RedisResult<Value>
    where
        C: crate::aio::ConnectionLike,
    {
        let mut resp = con
            .req_packed_commands(self, self.commands.len() + 1, 1)
            .await?;
        match resp.pop() {
            Some(Value::Nil) => Ok(Value::Nil),
            Some(Value::Array(items)) => Ok(self.make_pipeline_results(items)),
            _ => Err((
                ErrorKind::ResponseError,
                "Invalid response when parsing multi response",
            )
                .into()),
        }
    }

    /// Async version of `query`.
    #[inline]
    #[cfg(feature = "aio")]
    pub async fn query_async<C, T: FromRedisValue>(&self, con: &mut C) -> RedisResult<T>
    where
        C: crate::aio::ConnectionLike,
    {
        let v = if self.commands.is_empty() {
            return from_owned_redis_value(Value::Array(vec![]));
        } else if self.transaction_mode {
            self.execute_transaction_async(con).await?
        } else {
            self.execute_pipelined_async(con).await?
        };
        from_owned_redis_value(v)
    }

    /// Shortcut to `query()` that does not return a value.
    #[inline]
    pub fn execute(&self, con: &mut dyn ConnectionLike) {
        self.query::<()>(con).unwrap();
    }

    /// Returns whether the pipeline is in transaction (atomic) mode.
    pub fn is_atomic(&self) -> bool {
        self.transaction_mode
    }

    /// Returns the number of commands in the pipeline.
    pub fn len(&self) -> usize {
        self.commands.len()
    }

    /// Returns `true` if the pipeline contains no commands.
    pub fn is_empty(&self) -> bool {
        self.commands.is_empty()
    }

    /// Returns the command at the given index, or `None` if the index is out of bounds.
    pub fn get_command(&self, index: usize) -> Option<&Arc<Cmd>> {
        self.commands.get(index)
    }
}

fn encode_pipeline(cmds: &[Arc<Cmd>], atomic: bool) -> Vec<u8> {
    let mut rv = vec![];
    write_pipeline(&mut rv, cmds, atomic);
    rv
}

fn write_pipeline(rv: &mut Vec<u8>, cmds: &[Arc<Cmd>], atomic: bool) {
    let cmds_len = cmds.iter().map(cmd_len_arc).sum();

    if atomic {
        let multi = Arc::new(cmd("MULTI"));
        let exec = Arc::new(cmd("EXEC"));
        rv.reserve(cmd_len(&*multi) + cmd_len(&*exec) + cmds_len);

        multi.write_packed_command_preallocated(rv);
        for cmd in cmds {
            cmd.write_packed_command_preallocated(rv);
        }
        exec.write_packed_command_preallocated(rv);
    } else {
        rv.reserve(cmds_len);
        for cmd in cmds {
            cmd.write_packed_command_preallocated(rv);
        }
    }
}

// Macro to implement shared methods between Pipeline and ClusterPipeline
macro_rules! implement_pipeline_commands {
    ($struct_name:ident) => {
        impl $struct_name {
            /// Adds a command to the pipeline. The command is wrapped in an `Arc`.
            #[inline]
            pub fn add_command(&mut self, cmd: Arc<Cmd>) -> &mut Self {
                self.commands.push(cmd);
                self
            }

            /// Starts a new command. Functions such as `arg` then become available.
            #[inline]
            pub fn cmd(&mut self, name: &str) -> &mut Self {
                self.add_command(Arc::new(cmd(name)))
            }

            /// Returns an iterator over all the commands currently in the pipeline.
            pub fn cmd_iter(&self) -> impl Iterator<Item = &Arc<Cmd>> {
                self.commands.iter()
            }

            /// Instructs the pipeline to ignore the return value of this command.
            #[inline]
            pub fn ignore(&mut self) -> &mut Self {
                match self.commands.len() {
                    0 => true,
                    x => self.ignored_commands.insert(x - 1),
                };
                self
            }

            /// Adds an argument to the last started command.
            #[inline]
            pub fn arg<T: ToRedisArgs>(&mut self, arg: T) -> &mut Self {
                {
                    let cmd = self.get_last_command();
                    cmd.arg(arg);
                }
                self
            }

            /// Clears the pipeline's internal state.
            #[inline]
            pub fn clear(&mut self) {
                self.commands.clear();
                self.ignored_commands.clear();
            }

            /// Returns a mutable reference to the last command.
            /// Uses `Arc::make_mut` to allow mutation if the command is uniquely owned.
            #[inline]
            fn get_last_command(&mut self) -> &mut Cmd {
                let idx = self
                    .commands
                    .len()
                    .checked_sub(1)
                    .expect("No command on stack");
                Arc::make_mut(&mut self.commands[idx])
            }

            fn make_pipeline_results(&self, resp: Vec<Value>) -> Value {
                let mut rv = Vec::with_capacity(resp.len() - self.ignored_commands.len());
                for (idx, result) in resp.into_iter().enumerate() {
                    if !self.ignored_commands.contains(&idx) {
                        rv.push(result);
                    }
                }
                Value::Array(rv)
            }
        }

        impl Default for $struct_name {
            fn default() -> Self {
                Self::new()
            }
        }
    };
}

implement_pipeline_commands!(Pipeline);
