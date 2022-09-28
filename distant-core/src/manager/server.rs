use crate::{
    ChannelId, ConnectionId, ConnectionInfo, ConnectionList, Destination, ManagerCapabilities,
    ManagerRequest, ManagerResponse, Map,
};
use async_trait::async_trait;
use distant_net::{
    Client, Listener, MpscListener, Request, Response, ServerCtx, ServerExt, ServerHandler,
};
use log::*;
use std::{collections::HashMap, io, sync::Arc};
use tokio::{
    sync::{mpsc, Mutex, RwLock},
    task::JoinHandle,
};

mod config;
pub use config::*;

mod connection;
pub use connection::*;

mod ext;
pub use ext::*;

mod handler;
pub use handler::*;

mod r#ref;
pub use r#ref::*;

/// Represents a manager of multiple distant server connections
pub struct DistantManager {
    /// Receives authentication clients to feed into local data of server
    auth_client_rx: Mutex<mpsc::Receiver<AuthClient>>,

    /// Configuration settings for the server
    config: DistantManagerConfig,

    /// Mapping of connection id -> connection
    connections: RwLock<HashMap<ConnectionId, DistantManagerConnection>>,

    /// Handlers for launch requests
    launch_handlers: Arc<RwLock<HashMap<String, BoxedLaunchHandler>>>,

    /// Handlers for connect requests
    connect_handlers: Arc<RwLock<HashMap<String, BoxedConnectHandler>>>,

    /// Primary task of server
    task: JoinHandle<()>,
}

impl DistantManager {
    /// Initializes a new instance of [`DistantManagerServer`] using the provided [`UntypedTransport`]
    pub fn start<L, T>(
        mut config: DistantManagerConfig,
        mut listener: L,
    ) -> io::Result<DistantManagerRef>
    where
        L: Listener<Output = T> + 'static,
        T: IntoSplit + Send + 'static,
        T::Read: UntypedTransportRead + 'static,
        T::Write: UntypedTransportWrite + 'static,
    {
        let (conn_tx, mpsc_listener) = MpscListener::channel(config.connection_buffer_size);
        let (auth_client_tx, auth_client_rx) = mpsc::channel(1);

        // Spawn task that uses our input listener to get both auth and manager events,
        // forwarding manager events to the internal mpsc listener
        let task = tokio::spawn(async move {
            while let Ok(transport) = listener.accept().await {
                let DistantManagerRouter {
                    auth_transport,
                    manager_transport,
                    ..
                } = DistantManagerRouter::new(transport);

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

        let launch_handlers = Arc::new(RwLock::new(config.launch_handlers.drain().collect()));
        let weak_launch_handlers = Arc::downgrade(&launch_handlers);
        let connect_handlers = Arc::new(RwLock::new(config.connect_handlers.drain().collect()));
        let weak_connect_handlers = Arc::downgrade(&connect_handlers);
        let server_ref = Self {
            auth_client_rx: Mutex::new(auth_client_rx),
            config,
            launch_handlers,
            connect_handlers,
            connections: RwLock::new(HashMap::new()),
            task,
        }
        .start(mpsc_listener)?;

        Ok(DistantManagerRef {
            launch_handlers: weak_launch_handlers,
            connect_handlers: weak_connect_handlers,
            inner: server_ref,
        })
    }

    /// Launches a new server at the specified `destination` using the given `options` information
    /// and authentication client (if needed) to retrieve additional information needed to
    /// enter the destination prior to starting the server, returning the destination of the
    /// launched server
    async fn launch(
        &self,
        destination: Destination,
        options: Map,
        auth: Option<&mut AuthClient>,
    ) -> io::Result<Destination> {
        let auth = auth.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Other,
                "Authentication client not initialized",
            )
        })?;

        let scheme = match destination.scheme.as_deref() {
            Some(scheme) => {
                trace!("Using scheme {}", scheme);
                scheme
            }
            None => {
                trace!(
                    "Using fallback scheme of {}",
                    self.config.launch_fallback_scheme.as_str()
                );
                self.config.launch_fallback_scheme.as_str()
            }
        }
        .to_lowercase();

        let credentials = {
            let lock = self.launch_handlers.read().await;
            let handler = lock.get(&scheme).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("No launch handler registered for {}", scheme),
                )
            })?;
            handler.launch(&destination, &options, auth).await?
        };

        Ok(credentials)
    }

    /// Connects to a new server at the specified `destination` using the given `options` information
    /// and authentication client (if needed) to retrieve additional information needed to
    /// establish the connection to the server
    async fn connect(
        &self,
        destination: Destination,
        options: Map,
        auth: Option<&mut AuthClient>,
    ) -> io::Result<ConnectionId> {
        let auth = auth.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Other,
                "Authentication client not initialized",
            )
        })?;

        let scheme = match destination.scheme.as_deref() {
            Some(scheme) => {
                trace!("Using scheme {}", scheme);
                scheme
            }
            None => {
                trace!(
                    "Using fallback scheme of {}",
                    self.config.connect_fallback_scheme.as_str()
                );
                self.config.connect_fallback_scheme.as_str()
            }
        }
        .to_lowercase();

        let (writer, reader) = {
            let lock = self.connect_handlers.read().await;
            let handler = lock.get(&scheme).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("No connect handler registered for {}", scheme),
                )
            })?;
            handler.connect(&destination, &options, auth).await?
        };

        let connection = DistantManagerConnection::new(destination, options, writer, reader);
        let id = connection.id;
        self.connections.write().await.insert(id, connection);
        Ok(id)
    }

    /// Retrieves the list of supported capabilities for this manager
    async fn capabilities(&self) -> io::Result<ManagerCapabilities> {
        Ok(ManagerCapabilities::all())
    }

    /// Retrieves information about the connection to the server with the specified `id`
    async fn info(&self, id: ConnectionId) -> io::Result<ConnectionInfo> {
        match self.connections.read().await.get(&id) {
            Some(connection) => Ok(ConnectionInfo {
                id: connection.id,
                destination: connection.destination.clone(),
                options: connection.options.clone(),
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
    async fn kill(&self, id: ConnectionId) -> io::Result<()> {
        match self.connections.write().await.remove(&id) {
            Some(_) => Ok(()),
            None => Err(io::Error::new(
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
    auth_client: Option<Mutex<AuthClient>>,

    /// Holds on to open channels feeding data back from a server to some connected client,
    /// enabling us to cancel the tasks on demand
    channels: RwLock<HashMap<ChannelId, DistantManagerChannel>>,
}

#[async_trait]
impl ServerHandler for DistantManager {
    type Request = ManagerRequest;
    type Response = ManagerResponse;
    type LocalData = DistantManagerServerConnection;

    async fn on_accept(&self, local_data: &mut Self::LocalData) {
        local_data.auth_client = self
            .auth_client_rx
            .lock()
            .await
            .recv()
            .await
            .map(Mutex::new);

        // Enable jit handshake
        if let Some(auth_client) = local_data.auth_client.as_ref() {
            auth_client.lock().await.set_jit_handshake(true);
        }
    }

    async fn on_request(&self, ctx: ServerCtx<Self::Request, Self::Response, Self::LocalData>) {
        let ServerCtx {
            connection_id,
            request,
            reply,
            local_data,
        } = ctx;

        let response = match request.payload {
            ManagerRequest::Capabilities {} => match self.capabilities().await {
                Ok(supported) => ManagerResponse::Capabilities { supported },
                Err(x) => ManagerResponse::Error(x.into()),
            },
            ManagerRequest::Launch {
                destination,
                options,
            } => {
                let mut auth = match local_data.auth_client.as_ref() {
                    Some(client) => Some(client.lock().await),
                    None => None,
                };

                match self
                    .launch(*destination, options, auth.as_deref_mut())
                    .await
                {
                    Ok(destination) => ManagerResponse::Launched { destination },
                    Err(x) => ManagerResponse::Error(x.into()),
                }
            }
            ManagerRequest::Connect {
                destination,
                options,
            } => {
                let mut auth = match local_data.auth_client.as_ref() {
                    Some(client) => Some(client.lock().await),
                    None => None,
                };

                match self
                    .connect(*destination, options, auth.as_deref_mut())
                    .await
                {
                    Ok(id) => ManagerResponse::Connected { id },
                    Err(x) => ManagerResponse::Error(x.into()),
                }
            }
            ManagerRequest::OpenChannel { id } => match self.connections.read().await.get(&id) {
                Some(connection) => match connection.open_channel(reply.clone()).await {
                    Ok(channel) => {
                        let id = channel.id();
                        local_data.channels.write().await.insert(id, channel);
                        ManagerResponse::ChannelOpened { id }
                    }
                    Err(x) => ManagerResponse::Error(x.into()),
                },
                None => ManagerResponse::Error(
                    io::Error::new(io::ErrorKind::NotConnected, "Connection does not exist").into(),
                ),
            },
            ManagerRequest::Channel { id, request } => {
                match local_data.channels.read().await.get(&id) {
                    // TODO: For now, we are NOT sending back a response to acknowledge
                    //       a successful channel send. We could do this in order for
                    //       the client to listen for a complete send, but is it worth it?
                    Some(channel) => match channel.send(request).await {
                        Ok(_) => return,
                        Err(x) => ManagerResponse::Error(x.into()),
                    },
                    None => ManagerResponse::Error(
                        io::Error::new(
                            io::ErrorKind::NotConnected,
                            "Channel is not open or does not exist",
                        )
                        .into(),
                    ),
                }
            }
            ManagerRequest::CloseChannel { id } => {
                match local_data.channels.write().await.remove(&id) {
                    Some(channel) => match channel.close().await {
                        Ok(_) => ManagerResponse::ChannelClosed { id },
                        Err(x) => ManagerResponse::Error(x.into()),
                    },
                    None => ManagerResponse::Error(
                        io::Error::new(
                            io::ErrorKind::NotConnected,
                            "Channel is not open or does not exist",
                        )
                        .into(),
                    ),
                }
            }
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

                // Clear out handler state in order to trigger drops
                self.launch_handlers.write().await.clear();
                self.connect_handlers.write().await.clear();

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
    use distant_net::{
        AuthClient, FramedTransport, HeapAuthServer, InmemoryTransport, IntoSplit, MappedListener,
        OneshotListener, PlainCodec, ServerExt, ServerRef,
    };

    /// Create a new server, bypassing the start loop
    fn setup() -> DistantManager {
        let (_, rx) = mpsc::channel(1);
        DistantManager {
            auth_client_rx: Mutex::new(rx),
            config: Default::default(),
            connections: RwLock::new(HashMap::new()),
            launch_handlers: Arc::new(RwLock::new(HashMap::new())),
            connect_handlers: Arc::new(RwLock::new(HashMap::new())),
            task: tokio::spawn(async move {}),
        }
    }

    /// Creates a connected [`AuthClient`] with a launched auth server that blindly responds
    fn auth_client_server() -> (AuthClient, Box<dyn ServerRef>) {
        let (t1, t2) = FramedTransport::pair(1);
        let client = AuthClient::from(Client::from_framed_transport(t1).unwrap());

        // Create a server that does nothing, but will support
        let server = HeapAuthServer {
            on_challenge: Box::new(|_, _| Vec::new()),
            on_verify: Box::new(|_, _| false),
            on_info: Box::new(|_| ()),
            on_error: Box::new(|_, _| ()),
        }
        .start(MappedListener::new(OneshotListener::from_value(t2), |t| {
            t.into_split()
        }))
        .unwrap();

        (client, server)
    }

    fn dummy_distant_writer_reader() -> (BoxedDistantWriter, BoxedDistantReader) {
        setup_distant_writer_reader().0
    }

    /// Creates a writer & reader with a connected transport
    fn setup_distant_writer_reader() -> (
        (BoxedDistantWriter, BoxedDistantReader),
        FramedTransport<InmemoryTransport, PlainCodec>,
    ) {
        let (t1, t2) = FramedTransport::pair(1);
        let (writer, reader) = t1.into_split();
        ((Box::new(writer), Box::new(reader)), t2)
    }

    #[tokio::test]
    async fn launch_should_fail_if_destination_scheme_is_unsupported() {
        let server = setup();

        let destination = "scheme://host".parse::<Destination>().unwrap();
        let options = "".parse::<Map>().unwrap();
        let (mut auth, _auth_server) = auth_client_server();
        let err = server
            .launch(destination, options, Some(&mut auth))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{:?}", err);
    }

    #[tokio::test]
    async fn launch_should_fail_if_handler_tied_to_scheme_fails() {
        let server = setup();

        let handler: Box<dyn LaunchHandler> = Box::new(|_: &_, _: &_, _: &mut _| async {
            Err(io::Error::new(io::ErrorKind::Other, "test failure"))
        });

        server
            .launch_handlers
            .write()
            .await
            .insert("scheme".to_string(), handler);

        let destination = "scheme://host".parse::<Destination>().unwrap();
        let options = "".parse::<Map>().unwrap();
        let (mut auth, _auth_server) = auth_client_server();
        let err = server
            .launch(destination, options, Some(&mut auth))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert_eq!(err.to_string(), "test failure");
    }

    #[tokio::test]
    async fn launch_should_return_new_destination_on_success() {
        let server = setup();

        let handler: Box<dyn LaunchHandler> = {
            Box::new(|_: &_, _: &_, _: &mut _| async {
                Ok("scheme2://host2".parse::<Destination>().unwrap())
            })
        };

        server
            .launch_handlers
            .write()
            .await
            .insert("scheme".to_string(), handler);

        let destination = "scheme://host".parse::<Destination>().unwrap();
        let options = "key=value".parse::<Map>().unwrap();
        let (mut auth, _auth_server) = auth_client_server();
        let destination = server
            .launch(destination, options, Some(&mut auth))
            .await
            .unwrap();

        assert_eq!(
            destination,
            "scheme2://host2".parse::<Destination>().unwrap()
        );
    }

    #[tokio::test]
    async fn connect_should_fail_if_destination_scheme_is_unsupported() {
        let server = setup();

        let destination = "scheme://host".parse::<Destination>().unwrap();
        let options = "".parse::<Map>().unwrap();
        let (mut auth, _auth_server) = auth_client_server();
        let err = server
            .connect(destination, options, Some(&mut auth))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{:?}", err);
    }

    #[tokio::test]
    async fn connect_should_fail_if_handler_tied_to_scheme_fails() {
        let server = setup();

        let handler: Box<dyn ConnectHandler> = Box::new(|_: &_, _: &_, _: &mut _| async {
            Err(io::Error::new(io::ErrorKind::Other, "test failure"))
        });

        server
            .connect_handlers
            .write()
            .await
            .insert("scheme".to_string(), handler);

        let destination = "scheme://host".parse::<Destination>().unwrap();
        let options = "".parse::<Map>().unwrap();
        let (mut auth, _auth_server) = auth_client_server();
        let err = server
            .connect(destination, options, Some(&mut auth))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert_eq!(err.to_string(), "test failure");
    }

    #[tokio::test]
    async fn connect_should_return_id_of_new_connection_on_success() {
        let server = setup();

        let handler: Box<dyn ConnectHandler> =
            Box::new(|_: &_, _: &_, _: &mut _| async { Ok(dummy_distant_writer_reader()) });

        server
            .connect_handlers
            .write()
            .await
            .insert("scheme".to_string(), handler);

        let destination = "scheme://host".parse::<Destination>().unwrap();
        let options = "key=value".parse::<Map>().unwrap();
        let (mut auth, _auth_server) = auth_client_server();
        let id = server
            .connect(destination, options, Some(&mut auth))
            .await
            .unwrap();

        let lock = server.connections.read().await;
        let connection = lock.get(&id).unwrap();
        assert_eq!(connection.id, id);
        assert_eq!(connection.destination, "scheme://host");
        assert_eq!(connection.options, "key=value".parse().unwrap());
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

        let (writer, reader) = dummy_distant_writer_reader();
        let connection = DistantManagerConnection::new(
            "scheme://host".parse().unwrap(),
            "key=value".parse().unwrap(),
            writer,
            reader,
        );
        let id = connection.id;
        server.connections.write().await.insert(id, connection);

        let info = server.info(id).await.unwrap();
        assert_eq!(
            info,
            ConnectionInfo {
                id,
                destination: "scheme://host".parse().unwrap(),
                options: "key=value".parse().unwrap(),
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

        let (writer, reader) = dummy_distant_writer_reader();
        let connection = DistantManagerConnection::new(
            "scheme://host".parse().unwrap(),
            "key=value".parse().unwrap(),
            writer,
            reader,
        );
        let id_1 = connection.id;
        server.connections.write().await.insert(id_1, connection);

        let (writer, reader) = dummy_distant_writer_reader();
        let connection = DistantManagerConnection::new(
            "other://host2".parse().unwrap(),
            "key=value".parse().unwrap(),
            writer,
            reader,
        );
        let id_2 = connection.id;
        server.connections.write().await.insert(id_2, connection);

        let list = server.list().await.unwrap();
        assert_eq!(
            list.get(&id_1).unwrap(),
            &"scheme://host".parse::<Destination>().unwrap()
        );
        assert_eq!(
            list.get(&id_2).unwrap(),
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

        let (writer, reader) = dummy_distant_writer_reader();
        let connection = DistantManagerConnection::new(
            "scheme://host".parse().unwrap(),
            "key=value".parse().unwrap(),
            writer,
            reader,
        );
        let id = connection.id;
        server.connections.write().await.insert(id, connection);

        server.kill(id).await.unwrap();

        let lock = server.connections.read().await;
        assert!(!lock.contains_key(&id), "Connection still exists");
    }
}
