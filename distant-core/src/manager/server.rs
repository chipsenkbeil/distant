use crate::{
    ConnectionInfo, ConnectionList, Destination, DistantMsg, DistantRequestData,
    DistantResponseData, Extra, ManagerRequest, ManagerResponse,
};
use async_trait::async_trait;
use distant_net::{
    router, Auth, AuthClient, Client, IntoSplit, Listener, MpscListener, Request, Response,
    SerdeTransport, Server, ServerCtx, ServerExt,
};
use log::*;
use std::{
    collections::{hash_map::Entry, HashMap},
    io,
    sync::Arc,
};
use tokio::{
    sync::{mpsc, Mutex, RwLock},
    task::JoinHandle,
};

mod config;
pub use config::*;

mod connection;
pub use connection::*;

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
    auth_client_rx: Mutex<mpsc::Receiver<AuthClient>>,

    /// Configuration settings for the server
    config: DistantManagerServerConfig,

    /// Mapping of connection id -> connection
    connections: RwLock<HashMap<usize, DistantManagerConnection>>,

    /// Handlers for connect requests
    connect_handlers: Arc<RwLock<HashMap<String, Box<dyn ConnectHandler + Send + Sync>>>>,

    /// Primary task of server
    task: JoinHandle<()>,
}

impl DistantManagerServer {
    /// Initializes a new instance of [`DistantManagerServer`] using the provided [`SerdeTransport`]
    pub fn start<L, T>(
        mut listener: L,
        config: DistantManagerServerConfig,
    ) -> io::Result<DistantManagerServerRef>
    where
        L: Listener<Output = T> + 'static,
        T: SerdeTransport + 'static,
    {
        let (conn_tx, mpsc_listener) = MpscListener::channel(config.connection_buffer_size);
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
            auth_client_rx: Mutex::new(auth_client_rx),
            config,
            connect_handlers,
            connections: RwLock::new(HashMap::new()),
            task,
        }
        .start(mpsc_listener)?;

        Ok(DistantManagerServerRef {
            connect_handlers: weak_connect_handlers,
            inner: server_ref,
        })
    }

    /// Connects to a new server at the specified `destination` using the given `extra` information
    /// and authentication client (if needed) to retrieve additional information needed to
    /// establish the connection to the server
    async fn connect(
        &self,
        destination: Destination,
        extra: Extra,
        auth: &AuthClient,
    ) -> io::Result<usize> {
        let scheme = destination
            .scheme()
            .map(|scheme| scheme.as_str())
            .unwrap_or(self.config.fallback_scheme.as_str())
            .to_lowercase();

        let client = {
            let lock = self.connect_handlers.read().await;
            let handler = lock.get(&scheme).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("No scheme handler registered for {}", scheme),
                )
            })?;
            handler.connect(&destination, &extra, auth).await?
        };
        let id = rand::random();
        let connection = DistantManagerConnection {
            id,
            destination,
            extra,
            client,
        };
        self.connections.write().await.insert(id, connection);
        Ok(id)
    }

    /// Makes a request to the server with the specified `id`, returning the response
    async fn request(
        &self,
        id: usize,
        payload: DistantMsg<DistantRequestData>,
    ) -> io::Result<DistantMsg<DistantResponseData>> {
        let mut lock = self.connections.write().await;
        let connection = lock
            .get_mut(&id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotConnected, "No connection found"))?;
        let response = connection.client.send(payload).await?;
        Ok(response.payload)
    }

    /// Retrieves information about the connection to the server with the specified `id`
    async fn info(&self, id: usize) -> io::Result<ConnectionInfo> {
        match self.connections.read().await.get(&id) {
            Some(connection) => Ok(ConnectionInfo {
                id: connection.id,
                destination: connection.destination.clone(),
                extra: connection.extra.clone(),
            }),
            None => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "No connection found",
            )),
        }
    }

    /// Retrieves a list of connections to servers
    async fn list(&self) -> io::Result<ConnectionList> {
        Ok(ConnectionList(
            self.connections
                .read()
                .await
                .values()
                .map(|conn| (conn.id, conn.destination.clone()))
                .collect(),
        ))
    }

    /// Kills the connection to the server with the specified `id`
    async fn kill(&self, id: usize) -> io::Result<()> {
        match self.connections.write().await.entry(id) {
            Entry::Occupied(x) => {
                // Kill the client's tasks
                x.get().client.abort();

                // Remove the connection from our list
                let _ = x.remove();

                Ok(())
            }
            Entry::Vacant(_) => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "No connection found",
            )),
        }
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
        local_data.auth_client = self.auth_client_rx.lock().await.recv().await;
    }

    async fn on_request(&self, ctx: ServerCtx<Self::Request, Self::Response, Self::LocalData>) {
        let ServerCtx {
            connection_id,
            request,
            reply,
            local_data,
        } = ctx;

        let response = match request.payload {
            ManagerRequest::Connect { destination, extra } => {
                match self.connect(*destination, extra, local_data.auth()).await {
                    Ok(id) => ManagerResponse::Connected(id),
                    Err(x) => ManagerResponse::Error(x.into()),
                }
            }
            ManagerRequest::Request { id, payload } => match self.request(id, payload).await {
                Ok(payload) => ManagerResponse::Response(payload),
                Err(x) => ManagerResponse::Error(x.into()),
            },
            ManagerRequest::Info { id } => match self.info(id).await {
                Ok(info) => ManagerResponse::Info(info),
                Err(x) => ManagerResponse::Error(x.into()),
            },
            ManagerRequest::List => match self.list().await {
                Ok(list) => ManagerResponse::List(list),
                Err(x) => ManagerResponse::Error(x.into()),
            },
            ManagerRequest::Kill { id } => match self.kill(id).await {
                Ok(()) => ManagerResponse::Killed,
                Err(x) => ManagerResponse::Error(x.into()),
            },
            ManagerRequest::Shutdown => {
                if let Err(x) = reply.send(ManagerResponse::Shutdown).await {
                    error!("[Conn {}] {}", connection_id, x);
                }

                // Shutdown the primary server task
                self.task.abort();

                // TODO: Perform a graceful shutdown instead of this?
                //       Review https://tokio.rs/tokio/topics/shutdown
                std::process::exit(0);
            }
        };

        if let Err(x) = reply.send(response).await {
            error!("[Conn {}] {}", connection_id, x);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DistantClient;
    use async_trait::async_trait;
    use distant_net::{
        AuthClient, FramedTransport, InmemoryTransport, PlainCodec, SerdeTransportRead,
        SerdeTransportWrite,
    };

    /// Create a new server, bypassing the start loop
    fn setup() -> DistantManagerServer {
        let (_, rx) = mpsc::channel(1);
        DistantManagerServer {
            auth_client_rx: Mutex::new(rx),
            config: Default::default(),
            connections: RwLock::new(HashMap::new()),
            connect_handlers: Arc::new(RwLock::new(HashMap::new())),
            task: tokio::spawn(async move {}),
        }
    }

    /// Creates a dummy [`AuthClient`]
    fn dummy_auth_client() -> AuthClient {
        let (transport, _) = FramedTransport::pair(1);
        AuthClient::from(Client::from_framed_transport(transport).unwrap())
    }

    /// Creates a dummy [`DistantClient`]
    fn dummy_distant_client() -> DistantClient {
        setup_distant_client().0
    }

    /// Creates a [`DistantClient`] with a connected transport
    fn setup_distant_client() -> (
        DistantClient,
        FramedTransport<InmemoryTransport, PlainCodec>,
    ) {
        let (t1, t2) = FramedTransport::pair(1);
        (Client::from_framed_transport(t1).unwrap(), t2)
    }

    #[tokio::test]
    async fn connect_should_fail_if_destination_scheme_is_unsupported() {
        let server = setup();

        let destination = "scheme://host".parse::<Destination>().unwrap();
        let extra = "".parse::<Extra>().unwrap();
        let auth = dummy_auth_client();
        let err = server.connect(destination, extra, &auth).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{:?}", err);
    }

    #[tokio::test]
    async fn connect_should_fail_if_handler_tied_to_scheme_fails() {
        let server = setup();

        struct TestConnectHandler;

        #[async_trait]
        impl ConnectHandler for TestConnectHandler {
            async fn connect(
                &self,
                _destination: &Destination,
                _extra: &Extra,
                _auth: &AuthClient,
            ) -> io::Result<DistantClient> {
                Err(io::Error::new(io::ErrorKind::Other, "test failure"))
            }
        }

        server
            .connect_handlers
            .write()
            .await
            .insert("scheme".to_string(), Box::new(TestConnectHandler));

        let destination = "scheme://host".parse::<Destination>().unwrap();
        let extra = "".parse::<Extra>().unwrap();
        let auth = dummy_auth_client();
        let err = server.connect(destination, extra, &auth).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert_eq!(err.to_string(), "test failure");
    }

    #[tokio::test]
    async fn connect_should_return_id_of_new_connection_on_success() {
        let server = setup();

        struct TestConnectHandler;

        #[async_trait]
        impl ConnectHandler for TestConnectHandler {
            async fn connect(
                &self,
                _destination: &Destination,
                _extra: &Extra,
                _auth: &AuthClient,
            ) -> io::Result<DistantClient> {
                Ok(dummy_distant_client())
            }
        }

        server
            .connect_handlers
            .write()
            .await
            .insert("scheme".to_string(), Box::new(TestConnectHandler));

        let destination = "scheme://host".parse::<Destination>().unwrap();
        let extra = "key=value".parse::<Extra>().unwrap();
        let auth = dummy_auth_client();
        let id = server.connect(destination, extra, &auth).await.unwrap();

        let lock = server.connections.read().await;
        let connection = lock.get(&id).unwrap();
        assert_eq!(connection.id, id);
        assert_eq!(connection.destination, "scheme://host".parse().unwrap());
        assert_eq!(connection.extra, "key=value".parse().unwrap());
    }

    #[tokio::test]
    async fn request_should_fail_if_no_connection_found_for_specified_id() {
        let server = setup();

        let payload = DistantMsg::Single(DistantRequestData::SystemInfo {});
        let err = server.request(999, payload).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotConnected, "{:?}", err);
    }

    #[tokio::test]
    async fn request_should_fail_if_connected_client_fails_when_sending_request() {
        let server = setup();

        let id = 999;
        server.connections.write().await.insert(
            id,
            DistantManagerConnection {
                id,
                destination: "".parse().unwrap(),
                extra: "".parse().unwrap(),
                client: dummy_distant_client(),
            },
        );

        let payload = DistantMsg::Single(DistantRequestData::SystemInfo {});
        let err = server.request(id, payload).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::ConnectionAborted, "{:?}", err);
    }

    #[tokio::test]
    async fn request_should_return_payload_of_response_on_success() {
        let server = setup();

        let (client, mut transport) = setup_distant_client();

        let transport_task = tokio::spawn(async move {
            let request = transport
                .read::<Request<DistantMsg<DistantRequestData>>>()
                .await
                .unwrap()
                .unwrap();

            transport
                .write(Response::new(
                    request.id,
                    DistantMsg::Single(DistantResponseData::SystemInfo(Default::default())),
                ))
                .await
                .unwrap();
        });

        let id = 999;
        server.connections.write().await.insert(
            id,
            DistantManagerConnection {
                id,
                destination: "".parse().unwrap(),
                extra: "".parse().unwrap(),
                client,
            },
        );

        let payload = DistantMsg::Single(DistantRequestData::SystemInfo {});
        let msg = server.request(id, payload).await.unwrap();
        assert_eq!(
            msg,
            DistantMsg::Single(DistantResponseData::SystemInfo(Default::default()))
        );
        transport_task.await.unwrap();
    }

    #[tokio::test]
    async fn info_should_fail_if_no_connection_found_for_specified_id() {
        let server = setup();

        let err = server.info(999).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotConnected, "{:?}", err);
    }

    #[tokio::test]
    async fn info_should_return_information_about_established_connection() {
        let server = setup();

        let id = 999;
        server.connections.write().await.insert(
            id,
            DistantManagerConnection {
                id,
                destination: "scheme://host".parse().unwrap(),
                extra: "key=value".parse().unwrap(),
                client: dummy_distant_client(),
            },
        );

        let info = server.info(id).await.unwrap();
        assert_eq!(
            info,
            ConnectionInfo {
                id,
                destination: "scheme://host".parse().unwrap(),
                extra: "key=value".parse().unwrap(),
            }
        );
    }

    #[tokio::test]
    async fn list_should_return_empty_connection_list_if_no_established_connections() {
        let server = setup();

        let list = server.list().await.unwrap();
        assert_eq!(list, ConnectionList(HashMap::new()));
    }

    #[tokio::test]
    async fn list_should_return_a_list_of_established_connections() {
        let server = setup();

        server.connections.write().await.insert(
            1,
            DistantManagerConnection {
                id: 1,
                destination: "scheme://host".parse().unwrap(),
                extra: "key=value".parse().unwrap(),
                client: dummy_distant_client(),
            },
        );

        server.connections.write().await.insert(
            2,
            DistantManagerConnection {
                id: 2,
                destination: "other://host2".parse().unwrap(),
                extra: "key=value".parse().unwrap(),
                client: dummy_distant_client(),
            },
        );

        let list = server.list().await.unwrap();
        assert_eq!(
            list.get(&1).unwrap(),
            &"scheme://host".parse::<Destination>().unwrap()
        );
        assert_eq!(
            list.get(&2).unwrap(),
            &"other://host2".parse::<Destination>().unwrap()
        );
    }

    #[tokio::test]
    async fn kill_should_fail_if_no_connection_found_for_specified_id() {
        let server = setup();

        let err = server.kill(999).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotConnected, "{:?}", err);
    }

    #[tokio::test]
    async fn kill_should_terminate_established_connection_and_remove_it_from_the_list() {
        let server = setup();

        let id = 999;
        server.connections.write().await.insert(
            id,
            DistantManagerConnection {
                id,
                destination: "scheme://host".parse().unwrap(),
                extra: "key=value".parse().unwrap(),
                client: dummy_distant_client(),
            },
        );

        let _ = server.kill(id).await.unwrap();

        let lock = server.connections.read().await;
        assert!(!lock.contains_key(&id), "Connection still exists");
    }
}
