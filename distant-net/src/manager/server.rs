use crate::{
    common::{authentication::msg::AuthenticationResponse, ConnectionId, Destination, Map},
    manager::{
        ConnectionInfo, ConnectionList, ManagerAuthenticationId, ManagerCapabilities,
        ManagerChannelId, ManagerRequest, ManagerResponse,
    },
    server::{Server, ServerCtx, ServerHandler},
};
use async_trait::async_trait;
use log::*;
use std::{collections::HashMap, io, sync::Arc};
use tokio::sync::{oneshot, RwLock};

mod authentication;
pub use authentication::*;

mod config;
pub use config::*;

mod connection;
pub use connection::*;

mod handler;
pub use handler::*;

/// Represents a manager of multiple server connections.
pub struct ManagerServer {
    /// Configuration settings for the server
    config: Config,

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

        let credentials = {
            let handler = self.config.launch_handlers.get(&scheme).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("No launch handler registered for {}", scheme),
                )
            })?;
            handler
                .launch(&destination, &options, &mut authenticator)
                .await?
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

        let client = {
            let handler = self.config.connect_handlers.get(&scheme).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("No connect handler registered for {}", scheme),
                )
            })?;
            handler
                .connect(&destination, &options, &mut authenticator)
                .await?
        };

        let connection = ManagerConnection::spawn(destination, options, client).await?;
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
    /// Holds on to open channels feeding data back from a server to some connected client,
    /// enabling us to cancel the tasks on demand
    channels: RwLock<HashMap<ManagerChannelId, ManagerChannel>>,
}

#[async_trait]
impl ServerHandler for ManagerServer {
    type Request = ManagerRequest;
    type Response = ManagerResponse;
    type LocalData = DistantManagerServerConnection;

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
                Err(x) => ManagerResponse::from(x),
            },
            ManagerRequest::Launch {
                destination,
                options,
            } => match self
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
            },
            ManagerRequest::Connect {
                destination,
                options,
            } => match self
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
            },
            ManagerRequest::Authenticate { id, msg } => {
                match self.registry.write().await.remove(&id) {
                    Some(cb) => match cb.send(msg) {
                        Ok(_) => return,
                        Err(_) => ManagerResponse::Error {
                            description: "Unable to forward authentication callback".to_string(),
                        },
                    },
                    None => ManagerResponse::from(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Invalid authentication id",
                    )),
                }
            }
            ManagerRequest::OpenChannel { id } => match self.connections.read().await.get(&id) {
                Some(connection) => match connection.open_channel(reply.clone()) {
                    Ok(channel) => {
                        debug!("[Conn {id}] Channel {} has been opened", channel.id());
                        let id = channel.id();
                        local_data.channels.write().await.insert(id, channel);
                        ManagerResponse::ChannelOpened { id }
                    }
                    Err(x) => ManagerResponse::from(x),
                },
                None => ManagerResponse::from(io::Error::new(
                    io::ErrorKind::NotConnected,
                    "Connection does not exist",
                )),
            },
            ManagerRequest::Channel { id, request } => {
                match local_data.channels.read().await.get(&id) {
                    // TODO: For now, we are NOT sending back a response to acknowledge
                    //       a successful channel send. We could do this in order for
                    //       the client to listen for a complete send, but is it worth it?
                    Some(channel) => match channel.send(request) {
                        Ok(_) => return,
                        Err(x) => ManagerResponse::from(x),
                    },
                    None => ManagerResponse::from(io::Error::new(
                        io::ErrorKind::NotConnected,
                        "Channel is not open or does not exist",
                    )),
                }
            }
            ManagerRequest::CloseChannel { id } => {
                match local_data.channels.write().await.remove(&id) {
                    Some(channel) => match channel.close() {
                        Ok(_) => {
                            debug!("Channel {id} has been closed");
                            ManagerResponse::ChannelClosed { id }
                        }
                        Err(x) => ManagerResponse::from(x),
                    },
                    None => ManagerResponse::from(io::Error::new(
                        io::ErrorKind::NotConnected,
                        "Channel is not open or does not exist",
                    )),
                }
            }
            ManagerRequest::Info { id } => match self.info(id).await {
                Ok(info) => ManagerResponse::Info(info),
                Err(x) => ManagerResponse::from(x),
            },
            ManagerRequest::List => match self.list().await {
                Ok(list) => ManagerResponse::List(list),
                Err(x) => ManagerResponse::from(x),
            },
            ManagerRequest::Kill { id } => match self.kill(id).await {
                Ok(()) => ManagerResponse::Killed,
                Err(x) => ManagerResponse::from(x),
            },
        };

        if let Err(x) = reply.send(response).await {
            error!("[Conn {}] {}", connection_id, x);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::UntypedClient;
    use crate::common::FramedTransport;
    use crate::server::ServerReply;
    use crate::{boxed_connect_handler, boxed_launch_handler};
    use tokio::sync::mpsc;

    fn test_config() -> Config {
        Config {
            launch_fallback_scheme: "ssh".to_string(),
            connect_fallback_scheme: "distant".to_string(),
            connection_buffer_size: 100,
            user: false,
            launch_handlers: HashMap::new(),
            connect_handlers: HashMap::new(),
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
                tx: mpsc::channel(1).0,
            },
            registry: Arc::clone(&registry),
        };

        let server = ManagerServer {
            config,
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
    async fn launch_should_fail_if_handler_tied_to_scheme_fails() {
        let mut config = test_config();

        let handler = boxed_launch_handler!(|_a, _b, _c| {
            Err(io::Error::new(io::ErrorKind::Other, "test failure"))
        });

        config.launch_handlers.insert("scheme".to_string(), handler);

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

        let handler = boxed_launch_handler!(|_a, _b, _c| {
            Ok("scheme2://host2".parse::<Destination>().unwrap())
        });

        config.launch_handlers.insert("scheme".to_string(), handler);

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
    async fn connect_should_fail_if_handler_tied_to_scheme_fails() {
        let mut config = test_config();

        let handler = boxed_connect_handler!(|_a, _b, _c| {
            Err(io::Error::new(io::ErrorKind::Other, "test failure"))
        });

        config
            .connect_handlers
            .insert("scheme".to_string(), handler);

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

        let handler = boxed_connect_handler!(|_a, _b, _c| { Ok(detached_untyped_client()) });

        config
            .connect_handlers
            .insert("scheme".to_string(), handler);

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
