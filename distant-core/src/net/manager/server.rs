use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use crate::auth::msg::AuthenticationResponse;
use log::*;
use tokio::sync::{RwLock, oneshot};

use crate::net::common::{ConnectionId, Destination, Map};
use crate::net::manager::{
    ConnectionInfo, ConnectionList, ManagerAuthenticationId, ManagerChannelId, ManagerRequest,
    ManagerResponse, SemVer,
};
use crate::net::server::{RequestCtx, Server, ServerHandler};

mod authentication;
pub use authentication::*;

mod config;
pub use config::*;

mod connection;
pub use connection::*;

/// Represents a manager of multiple server connections.
pub struct ManagerServer {
    /// Configuration settings for the server
    config: Config,

    /// Holds on to open channels feeding data back from a server to some connected client,
    /// enabling us to cancel the tasks on demand
    channels: RwLock<HashMap<ManagerChannelId, ManagerChannel>>,

    /// Mapping of connection id -> connection
    connections: RwLock<HashMap<ConnectionId, ManagerConnection>>,

    /// Mapping of auth id -> callback
    registry:
        Arc<RwLock<HashMap<ManagerAuthenticationId, oneshot::Sender<AuthenticationResponse>>>>,
}

impl ManagerServer {
    /// Creates a new [`Server`] starting with a default configuration and no authentication
    /// methods. The provided `config` will be used to configure the launch and connect handlers
    /// for the server as well as provide other defaults.
    pub fn new(config: Config) -> Server<Self> {
        Server::new().handler(Self {
            config,
            channels: RwLock::new(HashMap::new()),
            connections: RwLock::new(HashMap::new()),
            registry: Arc::new(RwLock::new(HashMap::new())),
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
        mut authenticator: ManagerAuthenticator,
    ) -> io::Result<Destination> {
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

        let plugin = self.config.plugins.get(&scheme).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("No plugin registered for scheme '{scheme}'"),
            )
        })?;
        let destination = plugin
            .launch(&destination, &options, &mut authenticator)
            .await?;

        Ok(destination)
    }

    /// Connects to a new server at the specified `destination` using the given `options` information
    /// and authentication client (if needed) to retrieve additional information needed to
    /// establish the connection to the server
    async fn connect(
        &self,
        destination: Destination,
        options: Map,
        mut authenticator: ManagerAuthenticator,
    ) -> io::Result<ConnectionId> {
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

        let plugin = self.config.plugins.get(&scheme).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("No plugin registered for scheme '{scheme}'"),
            )
        })?;
        let client = plugin
            .connect(&destination, &options, &mut authenticator)
            .await?;

        let connection = ManagerConnection::spawn(destination, options, client).await?;
        let id = connection.id;
        self.connections.write().await.insert(id, connection);
        Ok(id)
    }

    /// Retrieves the manager's version.
    async fn version(&self) -> io::Result<SemVer> {
        env!("CARGO_PKG_VERSION").parse().map_err(io::Error::other)
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
            Some(connection) => {
                // Close any open channels
                if let Ok(ids) = connection.channel_ids().await {
                    let mut channels_lock = self.channels.write().await;
                    for id in ids {
                        if let Some(channel) = channels_lock.remove(&id)
                            && let Err(x) = channel.close()
                        {
                            error!("[Conn {id}] {x}");
                        }
                    }
                }

                // Make sure the connection is aborted so nothing new can happen
                debug!("[Conn {id}] Aborting");
                connection.abort();

                Ok(())
            }
            None => Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "No connection found",
            )),
        }
    }
}

impl ServerHandler for ManagerServer {
    type Request = ManagerRequest;
    type Response = ManagerResponse;

    async fn on_request(&self, ctx: RequestCtx<Self::Request, Self::Response>) {
        debug!("manager::on_request({ctx:?})");
        let RequestCtx {
            connection_id,
            request,
            reply,
        } = ctx;

        let response = match request.payload {
            ManagerRequest::Version => {
                debug!("Looking up version");
                match self.version().await {
                    Ok(version) => ManagerResponse::Version { version },
                    Err(x) => ManagerResponse::from(x),
                }
            }
            ManagerRequest::Launch {
                destination,
                options,
            } => {
                info!("Launching {destination} with {options}");
                match self
                    .launch(
                        *destination,
                        options,
                        ManagerAuthenticator {
                            reply: reply.clone(),
                            registry: Arc::clone(&self.registry),
                        },
                    )
                    .await
                {
                    Ok(destination) => ManagerResponse::Launched { destination },
                    Err(x) => ManagerResponse::from(x),
                }
            }
            ManagerRequest::Connect {
                destination,
                options,
            } => {
                info!("Connecting to {destination} with {options}");
                match self
                    .connect(
                        *destination,
                        options,
                        ManagerAuthenticator {
                            reply: reply.clone(),
                            registry: Arc::clone(&self.registry),
                        },
                    )
                    .await
                {
                    Ok(id) => ManagerResponse::Connected { id },
                    Err(x) => ManagerResponse::from(x),
                }
            }
            ManagerRequest::Authenticate { id, msg } => {
                trace!("Retrieving authentication callback registry");
                match self.registry.write().await.remove(&id) {
                    Some(cb) => {
                        trace!("Sending {msg:?} through authentication callback");
                        match cb.send(msg) {
                            Ok(_) => return,
                            Err(_) => ManagerResponse::Error {
                                description: "Unable to forward authentication callback"
                                    .to_string(),
                            },
                        }
                    }
                    None => ManagerResponse::from(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Invalid authentication id",
                    )),
                }
            }
            ManagerRequest::OpenChannel { id } => {
                debug!("Attempting to retrieve connection {id}");
                match self.connections.read().await.get(&id) {
                    Some(connection) => {
                        debug!("Opening channel through connection {id}");
                        match connection.open_channel(reply.clone()) {
                            Ok(channel) => {
                                info!("[Conn {id}] Channel {} has been opened", channel.id());
                                let id = channel.id();
                                self.channels.write().await.insert(id, channel);
                                ManagerResponse::ChannelOpened { id }
                            }
                            Err(x) => ManagerResponse::from(x),
                        }
                    }
                    None => ManagerResponse::from(io::Error::new(
                        io::ErrorKind::NotConnected,
                        "Connection does not exist",
                    )),
                }
            }
            ManagerRequest::Channel { id, request } => {
                debug!("Attempting to retrieve channel {id}");
                match self.channels.read().await.get(&id) {
                    // TODO: For now, we are NOT sending back a response to acknowledge
                    //       a successful channel send. We could do this in order for
                    //       the client to listen for a complete send, but is it worth it?
                    Some(channel) => {
                        debug!("Sending {request:?} through channel {id}");
                        match channel.send(request) {
                            Ok(_) => return,
                            Err(x) => ManagerResponse::from(x),
                        }
                    }
                    None => ManagerResponse::from(io::Error::new(
                        io::ErrorKind::NotConnected,
                        "Channel is not open or does not exist",
                    )),
                }
            }
            ManagerRequest::CloseChannel { id } => {
                debug!("Attempting to remove channel {id}");
                match self.channels.write().await.remove(&id) {
                    Some(channel) => {
                        debug!("Removed channel {}", channel.id());
                        match channel.close() {
                            Ok(_) => {
                                info!("Channel {id} has been closed");
                                ManagerResponse::ChannelClosed { id }
                            }
                            Err(x) => ManagerResponse::from(x),
                        }
                    }
                    None => ManagerResponse::from(io::Error::new(
                        io::ErrorKind::NotConnected,
                        "Channel is not open or does not exist",
                    )),
                }
            }
            ManagerRequest::Info { id } => {
                debug!("Attempting to retrieve information for connection {id}");
                match self.info(id).await {
                    Ok(info) => {
                        info!("Retrieved information for connection {id}");
                        ManagerResponse::Info(info)
                    }
                    Err(x) => ManagerResponse::from(x),
                }
            }
            ManagerRequest::List => {
                debug!("Attempting to retrieve the list of connections");
                match self.list().await {
                    Ok(list) => {
                        info!("Retrieved list of connections");
                        ManagerResponse::List(list)
                    }
                    Err(x) => ManagerResponse::from(x),
                }
            }
            ManagerRequest::Kill { id } => {
                debug!("Attempting to kill connection {id}");
                match self.kill(id).await {
                    Ok(()) => {
                        info!("Killed connection {id}");
                        ManagerResponse::Killed
                    }
                    Err(x) => ManagerResponse::from(x),
                }
            }
        };

        if let Err(x) = reply.send(response) {
            error!("[Conn {}] {}", connection_id, x);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::pin::Pin;

    use tokio::sync::mpsc;

    use super::*;
    use crate::auth::Authenticator;
    use crate::net::client::UntypedClient;
    use crate::net::common::FramedTransport;
    use crate::net::server::ServerReply;
    use crate::plugin::Plugin;

    /// Test plugin that returns an error from both launch and connect.
    struct FailPlugin {
        error_msg: String,
    }

    impl Plugin for FailPlugin {
        fn name(&self) -> &str {
            "fail"
        }

        fn connect<'a>(
            &'a self,
            _destination: &'a Destination,
            _options: &'a Map,
            _authenticator: &'a mut dyn Authenticator,
        ) -> Pin<Box<dyn std::future::Future<Output = io::Result<UntypedClient>> + Send + 'a>>
        {
            Box::pin(async { Err(io::Error::other(self.error_msg.clone())) })
        }

        fn launch<'a>(
            &'a self,
            _destination: &'a Destination,
            _options: &'a Map,
            _authenticator: &'a mut dyn Authenticator,
        ) -> Pin<Box<dyn std::future::Future<Output = io::Result<Destination>> + Send + 'a>>
        {
            Box::pin(async { Err(io::Error::other(self.error_msg.clone())) })
        }
    }

    /// Test plugin that returns a fixed destination from launch and a detached client from connect.
    struct SuccessPlugin {
        launch_dest: String,
    }

    impl Plugin for SuccessPlugin {
        fn name(&self) -> &str {
            "success"
        }

        fn connect<'a>(
            &'a self,
            _destination: &'a Destination,
            _options: &'a Map,
            _authenticator: &'a mut dyn Authenticator,
        ) -> Pin<Box<dyn std::future::Future<Output = io::Result<UntypedClient>> + Send + 'a>>
        {
            Box::pin(async { Ok(detached_untyped_client()) })
        }

        fn launch<'a>(
            &'a self,
            _destination: &'a Destination,
            _options: &'a Map,
            _authenticator: &'a mut dyn Authenticator,
        ) -> Pin<Box<dyn std::future::Future<Output = io::Result<Destination>> + Send + 'a>>
        {
            let dest = self.launch_dest.clone();
            Box::pin(async move { Ok(dest.parse::<Destination>().unwrap()) })
        }
    }

    fn test_config() -> Config {
        Config {
            launch_fallback_scheme: "ssh".to_string(),
            connect_fallback_scheme: "distant".to_string(),
            connection_buffer_size: 100,
            user: false,
            plugins: HashMap::new(),
        }
    }

    /// Create an untyped client that is detached such that reads and writes will fail
    fn detached_untyped_client() -> UntypedClient {
        UntypedClient::spawn_inmemory(FramedTransport::pair(1).0, Default::default())
    }

    /// Create a new server and authenticator
    fn setup(config: Config) -> (ManagerServer, ManagerAuthenticator) {
        let registry = Arc::new(RwLock::new(HashMap::new()));

        let authenticator = ManagerAuthenticator {
            reply: ServerReply {
                origin_id: format!("{}", rand::random::<u8>()),
                tx: mpsc::unbounded_channel().0,
            },
            registry: Arc::clone(&registry),
        };

        let server = ManagerServer {
            config,
            channels: RwLock::new(HashMap::new()),
            connections: RwLock::new(HashMap::new()),
            registry,
        };

        (server, authenticator)
    }

    #[tokio::test]
    async fn launch_should_fail_if_destination_scheme_is_unsupported() {
        let (server, authenticator) = setup(test_config());

        let destination = "scheme://host".parse::<Destination>().unwrap();
        let options = "".parse::<Map>().unwrap();
        let err = server
            .launch(destination, options, authenticator)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{:?}", err);
    }

    #[tokio::test]
    async fn launch_should_fail_if_plugin_tied_to_scheme_fails() {
        let mut config = test_config();

        let plugin = Arc::new(FailPlugin {
            error_msg: "test failure".to_string(),
        });
        config.plugins.insert("scheme".to_string(), plugin);

        let (server, authenticator) = setup(config);
        let destination = "scheme://host".parse::<Destination>().unwrap();
        let options = "".parse::<Map>().unwrap();
        let err = server
            .launch(destination, options, authenticator)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert_eq!(err.to_string(), "test failure");
    }

    #[tokio::test]
    async fn launch_should_return_new_destination_on_success() {
        let mut config = test_config();

        let plugin = Arc::new(SuccessPlugin {
            launch_dest: "scheme2://host2".to_string(),
        });
        config.plugins.insert("scheme".to_string(), plugin);

        let (server, authenticator) = setup(config);
        let destination = "scheme://host".parse::<Destination>().unwrap();
        let options = "key=value".parse::<Map>().unwrap();
        let destination = server
            .launch(destination, options, authenticator)
            .await
            .unwrap();

        assert_eq!(
            destination,
            "scheme2://host2".parse::<Destination>().unwrap()
        );
    }

    #[tokio::test]
    async fn connect_should_fail_if_destination_scheme_is_unsupported() {
        let (server, authenticator) = setup(test_config());

        let destination = "scheme://host".parse::<Destination>().unwrap();
        let options = "".parse::<Map>().unwrap();
        let err = server
            .connect(destination, options, authenticator)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{:?}", err);
    }

    #[tokio::test]
    async fn connect_should_fail_if_plugin_tied_to_scheme_fails() {
        let mut config = test_config();

        let plugin = Arc::new(FailPlugin {
            error_msg: "test failure".to_string(),
        });
        config.plugins.insert("scheme".to_string(), plugin);

        let (server, authenticator) = setup(config);
        let destination = "scheme://host".parse::<Destination>().unwrap();
        let options = "".parse::<Map>().unwrap();
        let err = server
            .connect(destination, options, authenticator)
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert_eq!(err.to_string(), "test failure");
    }

    #[tokio::test]
    async fn connect_should_return_id_of_new_connection_on_success() {
        let mut config = test_config();

        let plugin: Arc<dyn Plugin> = Arc::new(SuccessPlugin {
            launch_dest: String::new(),
        });
        config.plugins.insert("scheme".to_string(), plugin);

        let (server, authenticator) = setup(config);
        let destination = "scheme://host".parse::<Destination>().unwrap();
        let options = "key=value".parse::<Map>().unwrap();
        let id = server
            .connect(destination, options, authenticator)
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
        let (server, _) = setup(test_config());

        let err = server.info(999).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotConnected, "{:?}", err);
    }

    #[tokio::test]
    async fn info_should_return_information_about_established_connection() {
        let (server, _) = setup(test_config());

        let connection = ManagerConnection::spawn(
            "scheme://host".parse().unwrap(),
            "key=value".parse().unwrap(),
            detached_untyped_client(),
        )
        .await
        .unwrap();
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
        let (server, _) = setup(test_config());

        let list = server.list().await.unwrap();
        assert_eq!(list, ConnectionList(HashMap::new()));
    }

    #[tokio::test]
    async fn list_should_return_a_list_of_established_connections() {
        let (server, _) = setup(test_config());

        let connection = ManagerConnection::spawn(
            "scheme://host".parse().unwrap(),
            "key=value".parse().unwrap(),
            detached_untyped_client(),
        )
        .await
        .unwrap();
        let id_1 = connection.id;
        server.connections.write().await.insert(id_1, connection);

        let connection = ManagerConnection::spawn(
            "other://host2".parse().unwrap(),
            "key=value".parse().unwrap(),
            detached_untyped_client(),
        )
        .await
        .unwrap();
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
        let (server, _) = setup(test_config());

        let err = server.kill(999).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotConnected, "{:?}", err);
    }

    #[tokio::test]
    async fn kill_should_terminate_established_connection_and_remove_it_from_the_list() {
        let (server, _) = setup(test_config());

        let connection = ManagerConnection::spawn(
            "scheme://host".parse().unwrap(),
            "key=value".parse().unwrap(),
            detached_untyped_client(),
        )
        .await
        .unwrap();
        let id = connection.id;
        server.connections.write().await.insert(id, connection);

        server.kill(id).await.unwrap();

        let lock = server.connections.read().await;
        assert!(!lock.contains_key(&id), "Connection still exists");
    }
}
