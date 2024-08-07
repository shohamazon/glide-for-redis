/** Copyright Valkey GLIDE Project Contributors - SPDX Identifier: Apache-2.0 */
package glide.connectors.handlers;

import static glide.api.logging.Logger.Level.ERROR;

import glide.api.logging.Logger;
import io.netty.channel.ChannelHandlerContext;
import io.netty.channel.ChannelInboundHandlerAdapter;
import lombok.NonNull;
import lombok.RequiredArgsConstructor;
import response.ResponseOuterClass.Response;

/** Handler for inbound traffic though UDS. Used by Netty. */
@RequiredArgsConstructor
public class ReadHandler extends ChannelInboundHandlerAdapter {

    private final CallbackDispatcher callbackDispatcher;

    /** Submit responses from glide to an instance {@link CallbackDispatcher} to handle them. */
    @Override
    public void channelRead(@NonNull ChannelHandlerContext ctx, @NonNull Object msg)
            throws MessageHandler.MessageCallbackException {
        if (msg instanceof Response) {
            Response response = (Response) msg;
            callbackDispatcher.completeRequest(response);
            ctx.fireChannelRead(msg);
            return;
        }
        throw new RuntimeException("Unexpected message in socket");
    }

    /** Handles uncaught exceptions from {@link #channelRead(ChannelHandlerContext, Object)}. */
    @Override
    public void exceptionCaught(ChannelHandlerContext ctx, Throwable cause) throws Exception {
        if (cause instanceof MessageHandler.MessageCallbackException) {
            Logger.log(
                    ERROR,
                    "read handler",
                    () -> "=== Exception thrown from pubsub callback " + ctx,
                    cause.getCause());

            // Mimic the behavior of if this got thrown by a user thread. Print to stderr,
            cause.printStackTrace();

            // Unwrap. Only works for Exceptions and not Errors.
            throw ((MessageHandler.MessageCallbackException) cause).getCause();
        }
        Logger.log(ERROR, "read handler", () -> "=== exceptionCaught " + ctx, cause);
        callbackDispatcher.distributeClosingException(
                "An unhandled error while reading from UDS channel: " + cause);
    }
}
