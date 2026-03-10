use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;

use log::*;
use tokio::sync::{RwLock, broadcast, mpsc, oneshot};

use crate::auth::Authenticator;
use crate::auth::msg::*;
use crate::net::client::ConnectionState;
use crate::net::common::{ConnectionId, Map};
use crate::net::manager::{
    ConnectionInfo, ConnectionList, ManagerAuthenticationId, ManagerChannelId, ManagerRequest,
    ManagerResponse, SemVer,
};
use crate::net::server::{RequestCtx, Server, ServerHandler};
use crate::plugin::{Plugin, extract_scheme};

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

    /// Mapping of connection id -> connection.
    /// Wrapped in `Arc` so the background reconnection task can access connections
    /// without holding a borrow on `ManagerServer`.
    connections: Arc<RwLock<HashMap<ConnectionId, ManagerConnection>>>,

    /// Mapping of auth id -> callback
    registry:
        Arc<RwLock<HashMap<ManagerAuthenticationId, oneshot::Sender<AuthenticationResponse>>>>,

    /// Channel for sending connection death notifications from monitor tasks.
    /// Each [`ManagerConnection`] spawned by this server receives a clone to report
    /// when its underlying transport disconnects.
    death_tx: mpsc::UnboundedSender<ConnectionId>,

    /// Broadcast channel for sending connection state change events to subscribed clients.
    event_tx: broadcast::Sender<ManagerResponse>,
}

impl ManagerServer {
    /// Creates a new [`Server`] starting with a default configuration and no authentication
    /// methods. The provided `config` will be used to configure the launch and connect handlers
    /// for the server as well as provide other defaults.
    pub fn new(config: Config) -> Server<Self> {
        let (death_tx, mut death_rx) = mpsc::unbounded_channel();
        let (event_tx, _event_rx) = broadcast::channel(16);
        let connections = Arc::new(RwLock::new(HashMap::new()));

        // Spawn a background task that handles connection deaths.
        // When a connection dies, this task orchestrates reconnection using the
        // plugin's reconnect() method and retry strategy.
        {
            let connections = Arc::clone(&connections);
            let event_tx = event_tx.clone();
            let death_tx = death_tx.clone();
            let plugins = config.plugins.clone();
            let fallback_scheme = config.connect_fallback_scheme.clone();
            tokio::spawn(async move {
                while let Some(id) = death_rx.recv().await {
                    warn!("[Conn {id}] Connection death detected by manager");
                    handle_reconnection(
                        id,
                        &connections,
                        &plugins,
                        &fallback_scheme,
                        &death_tx,
                        &event_tx,
                    )
                    .await;
                }
            });
        }

        Server::new().handler(Self {
            config,
            channels: RwLock::new(HashMap::new()),
            connections,
            registry: Arc::new(RwLock::new(HashMap::new())),
            death_tx,
            event_tx,
        })
    }

    /// Launches a new server at the specified `destination` using the given `options` information
    /// and authentication client (if needed) to retrieve additional information needed to
    /// enter the destination prior to starting the server, returning the destination of the
    /// launched server
    async fn launch(
        &self,
        raw_destination: &str,
        options: Map,
        mut authenticator: ManagerAuthenticator,
    ) -> io::Result<crate::net::common::Destination> {
        let scheme = match extract_scheme(raw_destination) {
            Some(scheme) => {
                trace!("Using scheme {}", scheme);
                scheme.to_lowercase()
            }
            None => {
                trace!(
                    "Using fallback scheme of {}",
                    self.config.launch_fallback_scheme.as_str()
                );
                self.config.launch_fallback_scheme.to_lowercase()
            }
        };

        let plugin = self.config.plugins.get(&scheme).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("No plugin registered for scheme '{scheme}'"),
            )
        })?;
        let destination = plugin
            .launch(raw_destination, &options, &mut authenticator)
            .await?;

        Ok(destination)
    }

    /// Connects to a new server at the specified `destination` using the given `options` information
    /// and authentication client (if needed) to retrieve additional information needed to
    /// establish the connection to the server
    async fn connect(
        &self,
        raw_destination: &str,
        options: Map,
        mut authenticator: ManagerAuthenticator,
    ) -> io::Result<ConnectionId> {
        let scheme = match extract_scheme(raw_destination) {
            Some(scheme) => {
                trace!("Using scheme {}", scheme);
                scheme.to_lowercase()
            }
            None => {
                trace!(
                    "Using fallback scheme of {}",
                    self.config.connect_fallback_scheme.as_str()
                );
                self.config.connect_fallback_scheme.to_lowercase()
            }
        };

        let plugin = self.config.plugins.get(&scheme).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("No plugin registered for scheme '{scheme}'"),
            )
        })?;
        let client = plugin
            .connect(raw_destination, &options, &mut authenticator)
            .await?;

        let connection = ManagerConnection::spawn(
            raw_destination.to_string(),
            options,
            client,
            Some(self.death_tx.clone()),
        )
        .await?;
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
                .map(|conn| (conn.id, conn.destination.to_string()))
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

/// An [`Authenticator`] that fails all interactive authentication challenges.
///
/// Used during background reconnection where no user is available to answer
/// prompts. Plugins that rely solely on key files or ssh-agent will never
/// invoke the challenge/verify methods, so reconnection succeeds silently.
/// If the server requires interactive auth, reconnection fails immediately.
struct NonInteractiveAuthenticator;

impl Authenticator for NonInteractiveAuthenticator {
    fn initialize<'a>(
        &'a mut self,
        initialization: Initialization,
    ) -> Pin<Box<dyn Future<Output = io::Result<InitializationResponse>> + Send + 'a>> {
        // Accept whatever methods the server offers — let the plugin decide
        Box::pin(async move {
            Ok(InitializationResponse {
                methods: initialization.methods,
            })
        })
    }

    fn challenge<'a>(
        &'a mut self,
        _challenge: Challenge,
    ) -> Pin<Box<dyn Future<Output = io::Result<ChallengeResponse>> + Send + 'a>> {
        Box::pin(async {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "non-interactive reconnection cannot answer authentication challenges",
            ))
        })
    }

    fn verify<'a>(
        &'a mut self,
        _verification: Verification,
    ) -> Pin<Box<dyn Future<Output = io::Result<VerificationResponse>> + Send + 'a>> {
        // Auto-accept host verification during reconnection (already verified on first connect)
        Box::pin(async { Ok(VerificationResponse { valid: true }) })
    }

    fn info<'a>(
        &'a mut self,
        _info: Info,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn error<'a>(
        &'a mut self,
        _error: Error,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn start_method<'a>(
        &'a mut self,
        _start_method: StartMethod,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }

    fn finished<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async { Ok(()) })
    }
}

/// Sends a [`ConnectionState`] change notification through the event broadcast channel.
fn notify_state_change(
    event_tx: &broadcast::Sender<ManagerResponse>,
    id: ConnectionId,
    state: ConnectionState,
) {
    let _ = event_tx.send(ManagerResponse::ConnectionStateChanged { id, state });
}

/// Orchestrates reconnection for the connection with the given `id`.
///
/// Steps:
/// 1. Read connection info (destination, options) and look up the plugin by scheme.
/// 2. Check if the plugin supports reconnection (`reconnect_strategy != Fail`).
/// 3. Broadcast `Reconnecting` state to subscribers.
/// 4. Execute the plugin's `reconnect()` in a retry loop using the strategy.
/// 5. On success: hot-swap the old connection via `replace_client()`, broadcast `Connected`.
/// 6. On failure: broadcast `Disconnected`.
async fn handle_reconnection(
    id: ConnectionId,
    connections: &RwLock<HashMap<ConnectionId, ManagerConnection>>,
    plugins: &HashMap<String, Arc<dyn Plugin>>,
    fallback_scheme: &str,
    death_tx: &mpsc::UnboundedSender<ConnectionId>,
    event_tx: &broadcast::Sender<ManagerResponse>,
) {
    // Step 1: Read connection info without holding the lock across await points
    let (destination, options) = {
        let conns = connections.read().await;
        match conns.get(&id) {
            Some(conn) => (conn.destination.clone(), conn.options.clone()),
            None => {
                warn!("[Conn {id}] Reconnection aborted: connection not found");
                return;
            }
        }
    };

    // Look up the plugin by scheme
    let scheme = match extract_scheme(&destination) {
        Some(scheme) => scheme.to_lowercase(),
        None => fallback_scheme.to_lowercase(),
    };

    let plugin = match plugins.get(&scheme) {
        Some(plugin) => Arc::clone(plugin),
        None => {
            error!("[Conn {id}] Reconnection aborted: no plugin for scheme '{scheme}'");
            notify_state_change(event_tx, id, ConnectionState::Disconnected);
            return;
        }
    };

    // Step 2: Check reconnect strategy
    let strategy = plugin.reconnect_strategy();
    if strategy.is_fail() {
        info!("[Conn {id}] Plugin '{scheme}' does not support reconnection");
        notify_state_change(event_tx, id, ConnectionState::Disconnected);
        return;
    }

    // Step 3: Broadcast Reconnecting
    info!("[Conn {id}] Starting reconnection via plugin '{scheme}'");
    notify_state_change(event_tx, id, ConnectionState::Reconnecting);

    // Step 4: Retry loop using ReconnectStrategy's delay logic
    let mut previous_sleep = None;
    let mut current_sleep = strategy.initial_sleep_duration();
    let mut retries_remaining = strategy.max_retries();
    let timeout = strategy.timeout();
    let max_duration = strategy.max_duration();

    let mut last_err: Option<io::Error> = None;

    while retries_remaining.is_none() || retries_remaining > Some(0) {
        let mut authenticator = NonInteractiveAuthenticator;

        let result = match timeout {
            Some(t) => {
                match tokio::time::timeout(
                    t,
                    plugin.reconnect(&destination, &options, &mut authenticator),
                )
                .await
                {
                    Ok(r) => r,
                    Err(elapsed) => Err(elapsed.into()),
                }
            }
            None => {
                plugin
                    .reconnect(&destination, &options, &mut authenticator)
                    .await
            }
        };

        match result {
            Ok(new_client) => {
                // Step 5: Hot-swap the connection
                let mut conns = connections.write().await;
                if let Some(conn) = conns.get_mut(&id) {
                    match conn
                        .replace_client(new_client, Some(death_tx.clone()))
                        .await
                    {
                        Ok(()) => {
                            info!("[Conn {id}] Reconnection succeeded");
                            notify_state_change(event_tx, id, ConnectionState::Connected);
                            return;
                        }
                        Err(e) => {
                            error!("[Conn {id}] Failed to replace client after reconnect: {e}");
                            last_err = Some(e);
                        }
                    }
                } else {
                    warn!("[Conn {id}] Connection removed during reconnection");
                    return;
                }
            }
            Err(e) => {
                debug!("[Conn {id}] Reconnect attempt failed: {e}");
                last_err = Some(e);
            }
        }

        // Decrement remaining retries if we have a limit
        if let Some(remaining) = retries_remaining.as_mut()
            && *remaining > 0
        {
            *remaining -= 1;
        }

        // Sleep before next attempt
        tokio::time::sleep(current_sleep).await;

        // Update sleep duration using the strategy's backoff logic
        let next_sleep = strategy.adjust_sleep(previous_sleep, current_sleep);
        previous_sleep = Some(current_sleep);
        current_sleep = if let Some(duration) = max_duration {
            std::cmp::min(next_sleep, duration)
        } else {
            next_sleep
        };
    }

    // Step 6: All retries exhausted
    let err_msg = last_err
        .as_ref()
        .map(|e| e.to_string())
        .unwrap_or_else(|| "unknown error".to_string());
    error!("[Conn {id}] Reconnection failed after all retries: {err_msg}");
    notify_state_change(event_tx, id, ConnectionState::Disconnected);
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
                        &destination,
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
                        &destination,
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
            ManagerRequest::SubscribeConnectionEvents => {
                let mut event_rx = self.event_tx.subscribe();
                let reply_clone = reply.clone();
                tokio::spawn(async move {
                    while let Ok(event) = event_rx.recv().await {
                        if reply_clone.send(event).is_err() {
                            break;
                        }
                    }
                });
                ManagerResponse::SubscribedConnectionEvents
            }
            ManagerRequest::Reconnect { id } => {
                // Verify the connection exists before initiating reconnection
                let exists = self.connections.read().await.contains_key(&id);
                if !exists {
                    ManagerResponse::from(io::Error::new(
                        io::ErrorKind::NotConnected,
                        "No connection found",
                    ))
                } else {
                    info!("[Conn {id}] Manual reconnection requested");
                    // Spawn reconnection in the background so the response is immediate
                    let connections = Arc::clone(&self.connections);
                    let plugins = self.config.plugins.clone();
                    let fallback_scheme = self.config.connect_fallback_scheme.clone();
                    let death_tx = self.death_tx.clone();
                    let event_tx = self.event_tx.clone();
                    tokio::spawn(async move {
                        handle_reconnection(
                            id,
                            &connections,
                            &plugins,
                            &fallback_scheme,
                            &death_tx,
                            &event_tx,
                        )
                        .await;
                    });
                    ManagerResponse::ReconnectInitiated { id }
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

    use tokio::sync::{broadcast, mpsc};

    use super::*;
    use crate::auth::Authenticator;
    use crate::net::client::UntypedClient;
    use crate::net::common::{Destination, FramedTransport};
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
            _destination: &'a str,
            _options: &'a Map,
            _authenticator: &'a mut dyn Authenticator,
        ) -> Pin<Box<dyn std::future::Future<Output = io::Result<UntypedClient>> + Send + 'a>>
        {
            Box::pin(async { Err(io::Error::other(self.error_msg.clone())) })
        }

        fn launch<'a>(
            &'a self,
            _destination: &'a str,
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
            _destination: &'a str,
            _options: &'a Map,
            _authenticator: &'a mut dyn Authenticator,
        ) -> Pin<Box<dyn std::future::Future<Output = io::Result<UntypedClient>> + Send + 'a>>
        {
            Box::pin(async { Ok(detached_untyped_client()) })
        }

        fn launch<'a>(
            &'a self,
            _destination: &'a str,
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

        let (death_tx, _death_rx) = mpsc::unbounded_channel();
        let (event_tx, _event_rx) = broadcast::channel(16);

        let server = ManagerServer {
            config,
            channels: RwLock::new(HashMap::new()),
            connections: Arc::new(RwLock::new(HashMap::new())),
            registry,
            death_tx,
            event_tx,
        };

        (server, authenticator)
    }

    #[tokio::test]
    async fn launch_should_fail_if_destination_scheme_is_unsupported() {
        let (server, authenticator) = setup(test_config());

        let destination = "scheme://host";
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
        let destination = "scheme://host";
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
        let destination = "scheme://host";
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

        let destination = "scheme://host";
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
        let destination = "scheme://host";
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
        let destination = "scheme://host";
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
            "scheme://host",
            "key=value".parse().unwrap(),
            detached_untyped_client(),
            None,
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
                destination: "scheme://host".to_string(),
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
            "scheme://host",
            "key=value".parse().unwrap(),
            detached_untyped_client(),
            None,
        )
        .await
        .unwrap();
        let id_1 = connection.id;
        server.connections.write().await.insert(id_1, connection);

        let connection = ManagerConnection::spawn(
            "other://host2",
            "key=value".parse().unwrap(),
            detached_untyped_client(),
            None,
        )
        .await
        .unwrap();
        let id_2 = connection.id;
        server.connections.write().await.insert(id_2, connection);

        let list = server.list().await.unwrap();
        assert_eq!(list.get(&id_1).unwrap(), "scheme://host");
        assert_eq!(list.get(&id_2).unwrap(), "other://host2");
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
            "scheme://host",
            "key=value".parse().unwrap(),
            detached_untyped_client(),
            None,
        )
        .await
        .unwrap();
        let id = connection.id;
        server.connections.write().await.insert(id, connection);

        server.kill(id).await.unwrap();

        let lock = server.connections.read().await;
        assert!(!lock.contains_key(&id), "Connection still exists");
    }

    // ---------------------------------------------------------------
    // NonInteractiveAuthenticator tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn non_interactive_authenticator_initialize_should_echo_methods() {
        let mut auth = NonInteractiveAuthenticator;
        let init = crate::auth::msg::Initialization {
            methods: vec!["publickey".to_string(), "keyboard-interactive".to_string()],
        };
        let resp = auth.initialize(init).await.unwrap();
        assert_eq!(
            resp.methods,
            vec!["publickey".to_string(), "keyboard-interactive".to_string()]
        );
    }

    #[tokio::test]
    async fn non_interactive_authenticator_initialize_should_echo_empty_methods() {
        let mut auth = NonInteractiveAuthenticator;
        let init = crate::auth::msg::Initialization { methods: vec![] };
        let resp = auth.initialize(init).await.unwrap();
        assert!(resp.methods.is_empty());
    }

    #[tokio::test]
    async fn non_interactive_authenticator_initialize_should_echo_single_method() {
        let mut auth = NonInteractiveAuthenticator;
        let init = crate::auth::msg::Initialization {
            methods: vec!["none".to_string()],
        };
        let resp = auth.initialize(init).await.unwrap();
        assert_eq!(resp.methods, vec!["none".to_string()]);
    }

    #[tokio::test]
    async fn non_interactive_authenticator_challenge_should_return_permission_denied() {
        let mut auth = NonInteractiveAuthenticator;
        let challenge = crate::auth::msg::Challenge {
            questions: vec![crate::auth::msg::Question::new("Enter password")],
            options: std::collections::HashMap::new(),
        };
        let err = auth.challenge(challenge).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[tokio::test]
    async fn non_interactive_authenticator_challenge_should_have_descriptive_error_message() {
        let mut auth = NonInteractiveAuthenticator;
        let challenge = crate::auth::msg::Challenge {
            questions: vec![],
            options: std::collections::HashMap::new(),
        };
        let err = auth.challenge(challenge).await.unwrap_err();
        assert!(
            err.to_string()
                .contains("non-interactive reconnection cannot answer authentication challenges"),
            "Error message was: {}",
            err
        );
    }

    #[tokio::test]
    async fn non_interactive_authenticator_verify_should_return_valid_true() {
        let mut auth = NonInteractiveAuthenticator;
        let verification = crate::auth::msg::Verification {
            kind: crate::auth::msg::VerificationKind::Host,
            text: "ssh-ed25519 AAAA...".to_string(),
        };
        let resp = auth.verify(verification).await.unwrap();
        assert!(resp.valid);
    }

    #[tokio::test]
    async fn non_interactive_authenticator_verify_should_return_valid_for_unknown_kind() {
        let mut auth = NonInteractiveAuthenticator;
        let verification = crate::auth::msg::Verification {
            kind: crate::auth::msg::VerificationKind::Unknown,
            text: "something".to_string(),
        };
        let resp = auth.verify(verification).await.unwrap();
        assert!(resp.valid);
    }

    #[tokio::test]
    async fn non_interactive_authenticator_info_should_succeed() {
        let mut auth = NonInteractiveAuthenticator;
        let info = crate::auth::msg::Info {
            text: "Connecting to host...".to_string(),
        };
        auth.info(info).await.unwrap();
    }

    #[tokio::test]
    async fn non_interactive_authenticator_error_should_succeed() {
        let mut auth = NonInteractiveAuthenticator;
        let error = crate::auth::msg::Error::fatal("auth failed");
        auth.error(error).await.unwrap();
    }

    #[tokio::test]
    async fn non_interactive_authenticator_start_method_should_succeed() {
        let mut auth = NonInteractiveAuthenticator;
        let start = crate::auth::msg::StartMethod {
            method: "publickey".to_string(),
        };
        auth.start_method(start).await.unwrap();
    }

    #[tokio::test]
    async fn non_interactive_authenticator_finished_should_succeed() {
        let mut auth = NonInteractiveAuthenticator;
        auth.finished().await.unwrap();
    }

    // ---------------------------------------------------------------
    // notify_state_change tests
    // ---------------------------------------------------------------

    #[tokio::test]
    async fn notify_state_change_should_broadcast_connection_state_changed() {
        let (event_tx, mut event_rx) = broadcast::channel(16);
        let id: ConnectionId = 42;

        notify_state_change(&event_tx, id, ConnectionState::Reconnecting);

        let msg = event_rx.recv().await.unwrap();
        match msg {
            ManagerResponse::ConnectionStateChanged { id: recv_id, state } => {
                assert_eq!(recv_id, 42);
                assert_eq!(state, ConnectionState::Reconnecting);
            }
            other => panic!("Expected ConnectionStateChanged, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn notify_state_change_should_not_panic_with_no_subscribers() {
        let (event_tx, _) = broadcast::channel::<ManagerResponse>(16);
        // Drop the receiver before sending -- should not panic
        notify_state_change(&event_tx, 1, ConnectionState::Disconnected);
    }
}
