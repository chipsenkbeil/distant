use crate::{Listener, Request, Response, TypedAsyncRead, TypedAsyncWrite};
use log::*;
use std::{fmt::Debug, io, sync::Arc};
use tokio::{sync::mpsc, task::JoinHandle};

mod connection;
pub use connection::*;

mod context;
pub use context::*;

mod handler;
pub use handler::*;

mod state;
pub use state::*;

/// Represents a general-purpose server that leverages a [`Listener`] to receive new connections,
/// manages a global and connection-local state as part of [`ServerState`], and disperses
/// processing of incoming and outgoing data using [`ServerHandler`]
pub struct Server {
    task: JoinHandle<()>,
}

impl Server {
    /// Create a new server using the provided listener, handler, global data, and function to
    /// produce local data upon a new connection being established
    pub fn new<L, R, W, H, I, O, GlobalData, LocalData>(
        mut listener: L,
        handler: H,
        global_data: GlobalData,
        mut local_data: impl FnMut() -> LocalData + Send + 'static,
    ) -> io::Result<Self>
    where
        L: Listener<Output = (W, R)> + 'static,
        R: TypedAsyncRead<Request<I>> + Send + 'static,
        W: TypedAsyncWrite<Response<O>> + Send + 'static,
        H: ServerHandler<Request = I, Response = O, GlobalData = GlobalData, LocalData = LocalData>
            + Send
            + Sync
            + 'static,
        I: Debug + Send,
        O: Debug + Send + 'static,
        GlobalData: Send + Sync + 'static,
        LocalData: Send + Sync + 'static,
    {
        // let handler = Arc::new(handler);
        let state = Arc::new(ServerState::new(global_data));
        let handler = Arc::new(handler);

        let task = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((mut writer, mut reader)) => {
                        let mut connection = ServerConnection::new(local_data());
                        let connection_id = connection.id;

                        let handler = Arc::clone(&handler);
                        let state = Arc::clone(&state);

                        // Start a writer task that reads from a channel and forwards all
                        // data through the writer
                        let (tx, mut rx) = mpsc::channel::<Response<O>>(1);
                        connection.writer_task = Some(tokio::spawn(async move {
                            while let Some(data) = rx.recv().await {
                                trace!("[Conn {}] Sending {:?}", connection_id, data.payload);
                                if let Err(x) = writer.write(data).await {
                                    error!("[Conn {}] Failed to send {:?}", connection_id, x);
                                    break;
                                }
                            }
                        }));

                        // Start a reader task that reads requests and processes them
                        // using the provided handler
                        let reader_state = Arc::clone(&state);
                        connection.reader_task = Some(tokio::spawn(async move {
                            loop {
                                match reader.read().await {
                                    Ok(Some(request)) => {
                                        let reply = ServerCtxReply {
                                            origin_id: request.id,
                                            tx: tx.clone(),
                                        };

                                        let ctx = ServerCtx {
                                            connection_id,
                                            request,
                                            reply: reply.clone(),
                                            state: Arc::clone(&reader_state),
                                        };
                                        match handler.on_request(ctx).await {
                                            Ok(data) => {
                                                if let Err(x) = reply.send(data).await {
                                                    error!("[Conn {}] Connection closed and dropped message: {:?}", connection_id, x);
                                                    break;
                                                }
                                            }
                                            Err(x) => {
                                                error!(
                                                    "[Conn {}] Handler error: {}",
                                                    connection_id, x
                                                );
                                                continue;
                                            }
                                        }
                                    }
                                    Ok(None) => {
                                        debug!("[Conn {}] Connection closed", connection_id);
                                        break;
                                    }
                                    Err(x) => {
                                        error!(
                                            "[Conn {}] Connection failed: {:?}",
                                            connection_id, x
                                        );
                                        break;
                                    }
                                }
                            }
                        }));

                        state
                            .connections
                            .write()
                            .await
                            .insert(connection_id, connection);
                    }
                    Err(x) => {
                        error!("Server shutting down: {}", x);
                        break;
                    }
                }
            }
        });

        Ok(Self { task })
    }

    /// Aborts the server
    pub fn abort(&self) {
        self.task.abort()
    }

    /// Returns true if the server is no longer running
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{server_handler, IntoSplit, MpscTransport, TestListener};
    use tokio::sync::mpsc;

    pub struct TestServerHandler;

    server_handler! {
        name: TestServerHandler
        types: {
            Request = u16,
            Response = String,
            GlobalData = String,
            LocalData = (),
        }
        on_request: |ctx| {
            Err(io::Error::new(io::ErrorKind::Other, "Not implemented"))
        }
    }

    #[tokio::test]
    async fn should_invoke_handler_upon_receiving_a_request() {
        let handler = TestServerHandler;

        // Create a test listener where we will forward a connection
        let (tx, listener) = TestListener::channel(100);

        // Make bounded transport pair and send off one of them to act as our connection
        let (mut transport, connection) =
            MpscTransport::<Request<u16>, Response<String>>::pair(100);
        tx.send(connection.into_split())
            .await
            .expect("Failed to feed listener a connection");

        let server = Server::new(listener, handler, String::new(), || ());

        transport
            .write(Request::new(123))
            .await
            .expect("Failed to send request");

        // TODO: This hangs as we never send back a response because of the error
        //       Server needs to support having an error handler that is given a reply context
        //       so we have the option of replying, yet still provide a means to do nothing
        //
        //       This is because the protocol doesn't inherently have error handling built in.
        //       This is a user-level feature to provide an error type and send that back
        let response: Response<String> = transport.read().await.unwrap().unwrap();
        assert_eq!(response.payload, "hello");
    }
}
