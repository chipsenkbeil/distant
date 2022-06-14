use crate::{
    Listener, Request, Response, Server, ServerConnection, ServerCtxReply, ServerRequestCtx,
    ServerState, TypedAsyncRead, TypedAsyncWrite,
};
use log::*;
use serde::{de::DeserializeOwned, Serialize};
use std::{io, sync::Arc};
use tokio::{sync::mpsc, task::JoinHandle};

/// Extension trait to provide a reference implementation of starting a server
/// that will listen for new connections (exposed as [`TypedAsyncWrite`] and [`TypedAsyncRead`])
/// and process them using the [`Server`] implementation
pub trait ServerExt {
    type Request;
    type Response;

    /// Start a new server using the provided listener
    fn start<L, R, W>(listener: L) -> io::Result<ServerRef>
    where
        L: Listener<Output = (W, R)> + 'static,
        R: TypedAsyncRead<Request<Self::Request>> + Send + 'static,
        W: TypedAsyncWrite<Response<Self::Response>> + Send + 'static;
}

/// Reference to an actively-running server
pub struct ServerRef {
    task: JoinHandle<()>,
}

impl ServerRef {
    /// Returns true if the server is no longer running
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// Kills the internal task processing new inbound requests
    pub fn abort(&self) {
        self.task.abort();
    }
}

impl<S, Req, Res, Gdata, Ldata> ServerExt for S
where
    S: Server<Request = Req, Response = Res, GlobalData = Gdata, LocalData = Ldata>,
    Req: DeserializeOwned + Send + Sync,
    Res: Serialize + Send + 'static,
    Gdata: Default + Send + Sync + 'static,
    Ldata: Default + Send + Sync + 'static,
{
    type Request = Req;
    type Response = Res;

    fn start<L, R, W>(mut listener: L) -> io::Result<ServerRef>
    where
        L: Listener<Output = (W, R)> + 'static,
        R: TypedAsyncRead<Request<Self::Request>> + Send + 'static,
        W: TypedAsyncWrite<Response<Self::Response>> + Send + 'static,
    {
        // let handler = Arc::new(handler);
        let state = Arc::new(ServerState::new(Gdata::default()));

        let task = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((mut writer, mut reader)) => {
                        let mut connection = ServerConnection::new(Ldata::default());
                        let connection_id = connection.id;

                        let state = Arc::clone(&state);

                        // Start a writer task that reads from a channel and forwards all
                        // data through the writer
                        let (tx, mut rx) = mpsc::channel::<Response<Res>>(1);
                        connection.writer_task = Some(tokio::spawn(async move {
                            while let Some(data) = rx.recv().await {
                                // trace!("[Conn {}] Sending {:?}", connection_id, data.payload);
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

                                        let ctx = ServerRequestCtx {
                                            connection_id,
                                            request,
                                            reply: reply.clone(),
                                            state: Arc::clone(&reader_state),
                                        };
                                        match S::on_request(&ctx).await {
                                            Ok(_) => {}
                                            Err(x) => {
                                                error!(
                                                    "[Conn {}] Handler error: {}",
                                                    connection_id, x
                                                );
                                                S::on_error_with_request(&ctx, x).await;
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

        Ok(ServerRef { task })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IntoSplit, MpscTransport, TestListener};
    use async_trait::async_trait;

    pub struct TestServer;

    #[async_trait]
    impl Server for TestServer {
        type Request = u16;
        type Response = String;
        type GlobalData = String;
        type LocalData = ();

        async fn on_request(
            ctx: &ServerRequestCtx<
                Self::Request,
                Self::Response,
                Self::GlobalData,
                Self::LocalData,
            >,
        ) -> io::Result<()> {
            // Always send back "hello"
            ctx.reply.send("hello".to_string()).await.unwrap();

            Ok(())
        }
    }

    #[tokio::test]
    async fn should_invoke_handler_upon_receiving_a_request() {
        // Create a test listener where we will forward a connection
        let (tx, listener) = TestListener::channel(100);

        // Make bounded transport pair and send off one of them to act as our connection
        let (mut transport, connection) =
            MpscTransport::<Request<u16>, Response<String>>::pair(100);
        tx.send(connection.into_split())
            .await
            .expect("Failed to feed listener a connection");

        let _server = TestServer::start(listener).expect("Failed to start server");

        transport
            .write(Request::new(123))
            .await
            .expect("Failed to send request");

        let response: Response<String> = transport.read().await.unwrap().unwrap();
        assert_eq!(response.payload, "hello");
    }
}
