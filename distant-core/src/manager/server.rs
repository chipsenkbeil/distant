use super::{ManagerRequest, ManagerResponse};
use async_trait::async_trait;
use distant_net::{
    router, Auth, AuthClient, Client, IntoSplit, Listener, MpscListener, Request, Response,
    SerdeTransport, Server, ServerCtx, ServerExt,
};
use log::*;
use std::{collections::HashMap, io, sync::Arc};
use tokio::{
    sync::{mpsc, RwLock},
    task::JoinHandle,
};

const CONNECTION_BUFFER_SIZE: usize = 100;

mod config;
pub use config::*;

mod handler;
pub use handler::*;

mod r#ref;
pub use r#ref::*;

router!(DistantManagerServerRouter {
    auth_transport: Response<Auth> => Request<Auth>,
    manager_transport: Request<ManagerRequest> => Response<ManagerResponse>,
});

/// Represents a server that can connect to a remote distant manager
pub struct DistantManagerServer {
    /// Receives authentication clients to feed into local data of server
    auth_client_rx: mpsc::Receiver<AuthClient>,

    /// Handlers for connect requests
    connect_handlers: Arc<RwLock<HashMap<String, ConnectHandler>>>,

    /// Primary task of server
    task: JoinHandle<()>,
}

impl DistantManagerServer {
    /// Initializes a server using the provided [`SerdeTransport`]
    pub fn start<L, T>(
        mut listener: L,
        config: DistantManagerServerConfig,
    ) -> io::Result<DistantManagerServerRef>
    where
        L: Listener<Output = T> + 'static,
        T: SerdeTransport + 'static,
    {
        let (conn_tx, mpsc_listener) = MpscListener::channel(CONNECTION_BUFFER_SIZE);
        let (auth_client_tx, auth_client_rx) = mpsc::channel(1);

        // Spawn task that uses our input listener to get both auth and manager events,
        // forwarding manager events to the internal mpsc listener
        let task = tokio::spawn(async move {
            while let Ok(transport) = listener.accept().await {
                let DistantManagerServerRouter {
                    auth_transport,
                    manager_transport,
                    ..
                } = DistantManagerServerRouter::new(transport);

                let (writer, reader) = auth_transport.into_split();
                let client = match Client::new(writer, reader) {
                    Ok(client) => client,
                    Err(x) => {
                        error!("Creating auth client failed: {}", x);
                        continue;
                    }
                };
                let auth_client = AuthClient::from(client);

                // Forward auth client for new connection in server
                if auth_client_tx.send(auth_client).await.is_err() {
                    break;
                }

                // Forward connected and routed transport to server
                if conn_tx.send(manager_transport.into_split()).await.is_err() {
                    break;
                }
            }
        });

        let connect_handlers = Arc::new(RwLock::new(HashMap::new()));
        let weak_connect_handlers = Arc::downgrade(&connect_handlers);
        let server_ref = Self {
            auth_client_rx,
            connect_handlers,
            task,
        }
        .start(mpsc_listener)?;

        Ok(DistantManagerServerRef {
            connect_handlers: weak_connect_handlers,
            inner: server_ref,
        })
    }
}

#[derive(Default)]
pub struct DistantManagerServerConnection {
    /// Authentication client that manager can use when establishing a new connection
    /// and needing to get authentication details from the client to move forward
    auth_client: Option<AuthClient>,
}

impl DistantManagerServerConnection {
    /// Returns reference to authentication client associated with connection
    pub fn auth(&self) -> &AuthClient {
        // NOTE: We can unwrap as we know that the option should always be `Some(...)` by the time
        //       this function would be invoked
        self.auth_client.as_ref().unwrap()
    }
}

#[async_trait]
impl Server for DistantManagerServer {
    type Request = ManagerRequest;
    type Response = ManagerResponse;
    type LocalData = DistantManagerServerConnection;

    async fn on_accept(&self, local_data: &mut Self::LocalData) {
        local_data.auth_client = self.auth_client_rx.recv().await;
    }

    async fn on_request(&self, ctx: ServerCtx<Self::Request, Self::Response, Self::LocalData>) {
        let ServerCtx {
            connection_id,
            request,
            reply,
            local_data,
        } = ctx;

        match request.payload {
            ManagerRequest::Connect { destination, extra } => {
                let scheme = destination
                    .scheme()
                    .map(|scheme| scheme.as_str())
                    .unwrap_or("distant");

                if let Some(handler) = self.connect_handlers.read().await.get(scheme) {
                    match handler.do_connect(&destination, &extra).await {
                        Ok(client) => todo!("Store client and send back Connected(id)"),
                        Err(x) => todo!("Send an error back"),
                    }
                } else {
                    todo!("Send an error that the scheme is not supported");
                }
            }
            ManagerRequest::Request { id, payload } => {
                todo!();
            }
            ManagerRequest::Info { id } => {
                todo!();
            }
            ManagerRequest::Kill { id } => {
                todo!();
            }
            ManagerRequest::List => {
                todo!();
            }
            ManagerRequest::Shutdown => {
                // TODO: Actually perform shutdown
                if let Err(x) = reply.send(ManagerResponse::Shutdown).await {
                    error!("[Conn {}] {}", connection_id, x);
                }
            }
        }
    }
}
