use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use log::*;
use tokio::sync::{RwLock, broadcast, mpsc, oneshot};

use crate::auth::Authenticator;
use crate::auth::msg::*;
use crate::net::client::ConnectionState;
use crate::net::common::{ConnectionId, Map};
use crate::net::manager::data::Event;
use crate::net::manager::{
    ConnectionInfo, ConnectionList, EventTopic, ManagedTunnelId, ManagerAuthenticationId,
    ManagerChannelId, ManagerRequest, ManagerResponse, SemVer,
};
use crate::net::server::{RequestCtx, Server, ServerHandler, ServerReply};
use crate::plugin::{MountHandle, MountProbe, Plugin, extract_scheme};
use crate::protocol::{MountInfo, MountStatus};

mod authentication;
pub use authentication::*;

mod channel;
pub use channel::*;

mod config;
pub use config::*;

mod connection;
pub use connection::*;

mod tunnel;
pub use tunnel::*;

/// A mount whose lifecycle is managed by the manager process.
///
/// `info` is wrapped in `Arc<RwLock<...>>` so the per-mount monitor
/// task can publish state transitions without holding the outer
/// `self.mounts` write lock for the duration of the update. `handle`
/// is wrapped in `Arc<Mutex<Option<...>>>` so the monitor can call
/// `probe(&self)` while the unmount path retains exclusive access
/// (it `.lock().await.take()`s the inner value during teardown).
struct ManagedMount {
    info: Arc<RwLock<MountInfo>>,
    handle: Arc<tokio::sync::Mutex<Option<Box<dyn MountHandle>>>>,
    manager_channel: ManagerChannel,
    /// Per-mount monitor task. Aborted on unmount and on
    /// connection kill.
    monitor: tokio::task::JoinHandle<()>,
}

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

    /// Tunnels whose lifecycle is managed by this server process
    managed_tunnels: RwLock<HashMap<ManagedTunnelId, ManagedTunnel>>,

    /// Mounts whose lifecycle is managed by this server process
    mounts: RwLock<HashMap<u32, ManagedMount>>,

    /// Channel for sending connection death notifications from monitor tasks.
    /// Each [`ManagerConnection`] spawned by this server receives a clone to report
    /// when its underlying transport disconnects.
    death_tx: mpsc::UnboundedSender<ConnectionId>,

    /// Broadcast bus for [`Event`] push notifications. Subscribers
    /// (`ManagerRequest::Subscribe`) attach a `subscribe()` receiver
    /// and forward events that match their requested topics.
    event_tx: broadcast::Sender<Event>,
}

impl ManagerServer {
    /// Creates a new [`Server`] starting with a default configuration and no authentication
    /// methods. The provided `config` will be used to configure the launch and connect handlers
    /// for the server as well as provide other defaults.
    pub fn new(config: Config) -> Server<Self> {
        let (death_tx, mut death_rx) = mpsc::unbounded_channel();
        let (event_tx, _event_rx) = broadcast::channel::<Event>(16);
        let connections = Arc::new(RwLock::new(HashMap::new()));

        // Spawn a background task that handles connection deaths.
        // When a connection dies, this task orchestrates reconnection using
        // the plugin's reconnect() method and retry strategy.
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
            managed_tunnels: RwLock::new(HashMap::new()),
            mounts: RwLock::new(HashMap::new()),
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

                // Abort managed tunnels belonging to this connection
                let mut tunnels = self.managed_tunnels.write().await;
                let tunnel_ids: Vec<ManagedTunnelId> = tunnels
                    .values()
                    .filter(|t| t.connection_id == id)
                    .map(|t| t.id)
                    .collect();
                for tid in tunnel_ids {
                    if let Some(t) = tunnels.remove(&tid) {
                        debug!("[Conn {id}] Aborting managed tunnel {tid}");
                        t.abort();
                    }
                }
                drop(tunnels);

                // Tear down mounts belonging to this connection. Without
                // this loop, killing an SSH/Host/Docker connection that
                // had mounts on it would orphan the mounts in the map
                // with stale `Active` status.
                let mount_ids: Vec<u32> = {
                    let mounts = self.mounts.read().await;
                    let mut matching = Vec::new();
                    for (mount_id, mount) in mounts.iter() {
                        if mount.info.read().await.connection_id == id {
                            matching.push(*mount_id);
                        }
                    }
                    matching
                };
                if !mount_ids.is_empty() {
                    let removed: Vec<(u32, ManagedMount)> = {
                        let mut mounts = self.mounts.write().await;
                        mount_ids
                            .iter()
                            .filter_map(|mid| mounts.remove(mid).map(|m| (*mid, m)))
                            .collect()
                    };
                    for (mid, mount) in removed {
                        debug!("[Conn {id}] Aborting mount {mid}");
                        mount.monitor.abort();
                        let mut handle_slot = mount.handle.lock().await;
                        if let Some(mut handle) = handle_slot.take()
                            && let Err(e) = handle.unmount().await
                        {
                            warn!("[Conn {id}] Unmount of mount {mid} failed: {e}");
                        }
                        drop(handle_slot);
                        let _ = mount.manager_channel.close();
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

/// Sends an error response and returns early from `on_request`.
fn reply_err(reply: ServerReply<ManagerResponse>, conn_id: ConnectionId, err: io::Error) {
    let response = ManagerResponse::from(err);
    if let Err(x) = reply.send(response) {
        error!("[Conn {}] {}", conn_id, x);
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
        // Accept whatever methods the server offers — let the plugin decide.
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
        // Auto-accept host verification during reconnection (already verified on first connect).
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

/// Publishes a [`ConnectionState`] change as an [`Event::ConnectionState`]
/// on the broadcast bus. Subscribers (set up via `Subscribe { topics }`)
/// receive the event and forward it through their channels as
/// `ManagerResponse::Event { event }`.
fn publish_connection_state(
    event_tx: &broadcast::Sender<Event>,
    id: ConnectionId,
    state: ConnectionState,
) {
    let _ = event_tx.send(Event::ConnectionState { id, state });
}

/// Publishes a [`MountStatus`] change as an [`Event::MountState`] on the
/// broadcast bus.
fn publish_mount_state(event_tx: &broadcast::Sender<Event>, id: u32, state: MountStatus) {
    let _ = event_tx.send(Event::MountState { id, state });
}

/// Maps a [`MountProbe`] to a target [`MountStatus`] for the per-mount
/// monitor task.
///
/// Returns `None` when the probe should not change the current state
/// (e.g. `Healthy` while already `Active`, or `Degraded` which is
/// informational only).
fn probe_to_status(probe: MountProbe, current: &MountStatus) -> Option<MountStatus> {
    match probe {
        MountProbe::Healthy => match current {
            MountStatus::Active => None,
            // A successful probe restores any non-terminal state.
            MountStatus::Reconnecting | MountStatus::Disconnected => Some(MountStatus::Active),
            MountStatus::Failed { .. } => None,
        },
        MountProbe::Degraded(_) => None,
        MountProbe::Failed(reason) => match current {
            MountStatus::Failed { .. } => None,
            _ => Some(MountStatus::Failed { reason }),
        },
    }
}

/// Maps a [`ConnectionState`] change to a target [`MountStatus`].
///
/// Returns `None` when the connection state should not change the
/// mount's current state.
fn connection_state_to_mount_status(
    state: ConnectionState,
    current: &MountStatus,
) -> Option<MountStatus> {
    match (state, current) {
        // Connection coming back restores Reconnecting/Disconnected
        // mounts to Active. Active mounts are already there.
        (ConnectionState::Connected, MountStatus::Reconnecting)
        | (ConnectionState::Connected, MountStatus::Disconnected) => Some(MountStatus::Active),
        (ConnectionState::Connected, _) => None,
        // Reconnecting transitions Active mounts to Reconnecting; the
        // mount stays in any other state.
        (ConnectionState::Reconnecting, MountStatus::Active) => Some(MountStatus::Reconnecting),
        (ConnectionState::Reconnecting, _) => None,
        // Disconnected transitions Active/Reconnecting mounts to
        // Disconnected; Failed mounts are terminal.
        (ConnectionState::Disconnected, MountStatus::Active)
        | (ConnectionState::Disconnected, MountStatus::Reconnecting) => {
            Some(MountStatus::Disconnected)
        }
        (ConnectionState::Disconnected, _) => None,
    }
}

/// Per-mount monitor task body.
///
/// Polls the backend's [`MountHandle::probe`] every `interval` and
/// reacts to [`Event::ConnectionState`] events for `connection_id`
/// from the shared event bus. Each transition mutates
/// `info.write().await.status` and publishes a corresponding
/// [`Event::MountState`] event so subscribers can react.
///
/// Exits when:
/// - The mount handle is dropped (Mutex contains `None`) — the
///   unmount path takes ownership of the handle and the monitor sees
///   `None` on its next probe.
/// - The mount transitions to [`MountStatus::Failed`] — terminal,
///   no point continuing to poll.
async fn monitor_mount(
    mount_id: u32,
    connection_id: ConnectionId,
    info: Arc<RwLock<MountInfo>>,
    handle: Arc<tokio::sync::Mutex<Option<Box<dyn MountHandle>>>>,
    event_tx: broadcast::Sender<Event>,
    interval: Duration,
) {
    let mut event_rx = event_tx.subscribe();
    let mut ticker = tokio::time::interval(interval);
    // First tick fires immediately — burn it so we don't probe at t=0.
    ticker.tick().await;

    loop {
        tokio::select! {
            _ = ticker.tick() => {
                // Probe under the lock; the lock is held briefly and
                // probe() is non-async by trait contract.
                let probe = {
                    let guard = handle.lock().await;
                    match guard.as_ref() {
                        Some(h) => h.probe(),
                        None => {
                            debug!("[Mount {mount_id}] handle dropped, monitor exiting");
                            return;
                        }
                    }
                };

                let new_state = {
                    let info_guard = info.read().await;
                    probe_to_status(probe, &info_guard.status)
                };
                if let Some(new_state) = new_state {
                    let terminal = matches!(new_state, MountStatus::Failed { .. });
                    {
                        let mut info_guard = info.write().await;
                        info_guard.status = new_state.clone();
                    }
                    info!("[Mount {mount_id}] transitioned to {new_state:?}");
                    publish_mount_state(&event_tx, mount_id, new_state);
                    if terminal {
                        return;
                    }
                }
            }
            recv = event_rx.recv() => {
                let event = match recv {
                    Ok(e) => e,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("[Mount {mount_id}] monitor lagged {n} events");
                        continue;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        debug!("[Mount {mount_id}] event bus closed, monitor exiting");
                        return;
                    }
                };

                let Event::ConnectionState { id, state } = event else {
                    // We only care about connection state changes; the
                    // monitor publishes its own MountState events but
                    // shouldn't react to them.
                    continue;
                };
                if id != connection_id {
                    continue;
                }

                let new_state = {
                    let info_guard = info.read().await;
                    connection_state_to_mount_status(state, &info_guard.status)
                };
                if let Some(new_state) = new_state {
                    {
                        let mut info_guard = info.write().await;
                        info_guard.status = new_state.clone();
                    }
                    info!(
                        "[Mount {mount_id}] connection {id} → {state}, mount → {new_state:?}"
                    );
                    publish_mount_state(&event_tx, mount_id, new_state);
                }
            }
        }
    }
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
    event_tx: &broadcast::Sender<Event>,
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

    // Check if reconnection is disabled for this connection
    if options.get("no_reconnect").is_some_and(|v| v == "true") {
        info!("[Conn {id}] Reconnection disabled (--no-reconnect)");
        publish_connection_state(event_tx, id, ConnectionState::Disconnected);
        return;
    }

    // Look up the plugin by scheme
    let scheme = match extract_scheme(&destination) {
        Some(scheme) => scheme.to_lowercase(),
        None => fallback_scheme.to_lowercase(),
    };

    let plugin = match plugins.get(&scheme) {
        Some(plugin) => Arc::clone(plugin),
        None => {
            error!("[Conn {id}] Reconnection aborted: no plugin for scheme '{scheme}'");
            publish_connection_state(event_tx, id, ConnectionState::Disconnected);
            return;
        }
    };

    // Step 2: Check reconnect strategy
    let strategy = plugin.reconnect_strategy();
    if strategy.is_fail() {
        info!("[Conn {id}] Plugin '{scheme}' does not support reconnection");
        publish_connection_state(event_tx, id, ConnectionState::Disconnected);
        return;
    }

    // Step 3: Broadcast Reconnecting
    info!("[Conn {id}] Starting reconnection via plugin '{scheme}'");
    publish_connection_state(event_tx, id, ConnectionState::Reconnecting);

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
                            publish_connection_state(event_tx, id, ConnectionState::Connected);
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
    publish_connection_state(event_tx, id, ConnectionState::Disconnected);
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
            ManagerRequest::List { ref resources } => {
                if resources.contains(&crate::protocol::ResourceKind::Mount) {
                    debug!("Attempting to retrieve the list of mounts");
                    // Snapshot info Arcs under the outer lock, then release
                    // it before locking each individual info to avoid holding
                    // the outer write lock across .await.
                    let info_arcs: Vec<Arc<RwLock<MountInfo>>> = {
                        let mounts = self.mounts.read().await;
                        mounts.values().map(|m| Arc::clone(&m.info)).collect()
                    };
                    let mut mounts: Vec<MountInfo> = Vec::with_capacity(info_arcs.len());
                    for info_arc in info_arcs {
                        mounts.push(info_arc.read().await.clone());
                    }
                    info!("Retrieved {} mount(s)", mounts.len());
                    ManagerResponse::Mounts { mounts }
                } else {
                    debug!("Attempting to retrieve the list of connections");
                    match self.list().await {
                        Ok(list) => {
                            info!("Retrieved list of connections");
                            ManagerResponse::List(list)
                        }
                        Err(x) => ManagerResponse::from(x),
                    }
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
            ManagerRequest::ForwardTunnel {
                connection_id,
                bind_port,
                remote_host,
                remote_port,
            } => {
                debug!("Starting forward tunnel on connection {connection_id}");
                // Open the internal channel while briefly holding the read lock,
                // then drop the lock before doing async I/O (TCP bind).
                let internal = match self.connections.read().await.get(&connection_id) {
                    Some(connection) => match InternalRawChannel::open(connection) {
                        Ok(ic) => ic,
                        Err(x) => return reply_err(reply, connection_id, x),
                    },
                    None => {
                        return reply_err(
                            reply,
                            connection_id,
                            io::Error::new(
                                io::ErrorKind::NotConnected,
                                "Connection does not exist",
                            ),
                        );
                    }
                };
                // Lock dropped here — async tunnel setup proceeds without blocking connections
                match start_forward_tunnel(
                    internal,
                    connection_id,
                    bind_port,
                    remote_host,
                    remote_port,
                )
                .await
                {
                    Ok((managed, port)) => {
                        let id = managed.id;
                        self.managed_tunnels.write().await.insert(id, managed);
                        info!("Started forward tunnel {id} on port {port}");
                        ManagerResponse::ManagedTunnelStarted { id, port }
                    }
                    Err(x) => ManagerResponse::from(x),
                }
            }
            ManagerRequest::ReverseTunnel {
                connection_id,
                remote_port,
                local_host,
                local_port,
            } => {
                debug!("Starting reverse tunnel on connection {connection_id}");
                let internal = match self.connections.read().await.get(&connection_id) {
                    Some(connection) => match InternalRawChannel::open(connection) {
                        Ok(ic) => ic,
                        Err(x) => return reply_err(reply, connection_id, x),
                    },
                    None => {
                        return reply_err(
                            reply,
                            connection_id,
                            io::Error::new(
                                io::ErrorKind::NotConnected,
                                "Connection does not exist",
                            ),
                        );
                    }
                };
                match start_reverse_tunnel(
                    internal,
                    connection_id,
                    remote_port,
                    local_host,
                    local_port,
                )
                .await
                {
                    Ok((managed, port)) => {
                        let id = managed.id;
                        self.managed_tunnels.write().await.insert(id, managed);
                        info!("Started reverse tunnel {id} on port {port}");
                        ManagerResponse::ManagedTunnelStarted { id, port }
                    }
                    Err(x) => ManagerResponse::from(x),
                }
            }
            ManagerRequest::CloseManagedTunnel { id } => {
                debug!("Closing managed tunnel {id}");
                match self.managed_tunnels.write().await.remove(&id) {
                    Some(tunnel) => {
                        tunnel.abort();
                        info!("Closed managed tunnel {id}");
                        ManagerResponse::ManagedTunnelClosed
                    }
                    None => ManagerResponse::from(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("No managed tunnel with id {id}"),
                    )),
                }
            }
            ManagerRequest::ListManagedTunnels => {
                debug!("Listing managed tunnels");
                let tunnels = self
                    .managed_tunnels
                    .read()
                    .await
                    .values()
                    .map(|t| t.info.clone())
                    .collect();
                ManagerResponse::ManagedTunnels { tunnels }
            }
            ManagerRequest::Mount {
                connection_id,
                backend,
                mut config,
            } => {
                debug!("Mounting via plugin '{backend}' on connection {connection_id}");

                let plugin = match self.config.mount_plugins.get(&backend) {
                    Some(p) => Arc::clone(p),
                    None => {
                        return reply_err(
                            reply,
                            connection_id,
                            io::Error::new(
                                io::ErrorKind::NotFound,
                                format!("No mount plugin registered for backend '{backend}'"),
                            ),
                        );
                    }
                };

                // Open an internal channel and read the connection destination
                // while holding the read lock.
                let (internal, destination) =
                    match self.connections.read().await.get(&connection_id) {
                        Some(conn) => match InternalRawChannel::open(conn) {
                            Ok(ic) => (ic, conn.destination.clone()),
                            Err(e) => return reply_err(reply, connection_id, e),
                        },
                        None => {
                            return reply_err(
                                reply,
                                connection_id,
                                io::Error::new(
                                    io::ErrorKind::NotConnected,
                                    "Connection does not exist",
                                ),
                            );
                        }
                    };
                // Lock dropped — async mount proceeds without blocking connections.

                // Inject connection/runtime metadata into the config's extra
                // map for plugins that need it (e.g. FileProvider persists
                // this to a domain metadata file for the appex bootstrap).
                config
                    .extra
                    .insert("connection_id".into(), connection_id.to_string());
                config.extra.insert("destination".into(), destination);
                config
                    .extra
                    .insert("log_level".into(), log::max_level().to_string());

                let remote_root = config
                    .remote_root
                    .as_ref()
                    .map(|r| r.to_string())
                    .unwrap_or_default();
                let readonly = config.readonly;

                let (channel, manager_channel) = internal.into_parts();

                match plugin.mount(channel, config).await {
                    Ok(handle) => {
                        let mount_id: u32 = rand::random();
                        let mount_point = handle.mount_point().to_string();
                        let info = Arc::new(RwLock::new(MountInfo {
                            id: mount_id,
                            connection_id,
                            backend: backend.clone(),
                            mount_point: mount_point.clone(),
                            remote_root,
                            readonly,
                            status: MountStatus::Active,
                        }));
                        let handle = Arc::new(tokio::sync::Mutex::new(Some(handle)));

                        let monitor = tokio::spawn(monitor_mount(
                            mount_id,
                            connection_id,
                            Arc::clone(&info),
                            Arc::clone(&handle),
                            self.event_tx.clone(),
                            self.config.mount_health_interval,
                        ));

                        self.mounts.write().await.insert(
                            mount_id,
                            ManagedMount {
                                info,
                                handle,
                                manager_channel,
                                monitor,
                            },
                        );
                        info!("Mounted '{backend}' at '{mount_point}' (id={mount_id})");
                        ManagerResponse::Mounted {
                            id: mount_id,
                            mount_point,
                            backend,
                        }
                    }
                    Err(e) => {
                        error!("Mount via '{backend}' failed: {e}");
                        ManagerResponse::from(e)
                    }
                }
            }
            ManagerRequest::Unmount { ids } => {
                debug!("Unmounting {} mount(s)", ids.len());

                // Remove mounts under the lock, then unmount outside the lock
                // to avoid holding RwLockWriteGuard across .await.
                let removed: Vec<(u32, ManagedMount)> = {
                    let mut mounts = self.mounts.write().await;
                    ids.iter()
                        .filter_map(|id| mounts.remove(id).map(|m| (*id, m)))
                        .collect()
                };

                let mut unmounted = Vec::new();
                for (id, mount) in removed {
                    // Abort the monitor first so it stops poking the
                    // handle while we're tearing it down.
                    mount.monitor.abort();

                    let mut handle_slot = mount.handle.lock().await;
                    if let Some(mut handle) = handle_slot.take() {
                        if let Err(e) = handle.unmount().await {
                            warn!("Unmount of mount {id} failed: {e}");
                        } else {
                            info!("Unmounted mount {id}");
                        }
                    } else {
                        debug!("[Mount {id}] handle already taken (monitor or other unmounter)");
                    }
                    drop(handle_slot);

                    let _ = mount.manager_channel.close();
                    unmounted.push(id);
                }

                for id in &ids {
                    if !unmounted.contains(id) {
                        warn!("No mount with id {id}");
                    }
                }

                ManagerResponse::Unmounted { ids: unmounted }
            }
            ManagerRequest::Subscribe { topics } => {
                // Spawn a forwarder task that drains the broadcast bus and
                // wraps each `Event` in `ManagerResponse::Event` before
                // sending it back through the channel's reply stream. The
                // task exits when the reply stream closes (channel closed)
                // or when `recv()` errors (sender dropped).
                let mut event_rx = self.event_tx.subscribe();
                let reply_clone = reply.clone();
                let want_all = topics.contains(&EventTopic::All);
                let topics: std::collections::HashSet<EventTopic> = topics.into_iter().collect();
                tokio::spawn(async move {
                    while let Ok(event) = event_rx.recv().await {
                        if !want_all && !topics.contains(&event.topic()) {
                            continue;
                        }
                        if reply_clone.send(ManagerResponse::Event { event }).is_err() {
                            break;
                        }
                    }
                });
                ManagerResponse::Subscribed
            }
            ManagerRequest::Unsubscribe => {
                // The subscription forwarder task tied to this channel
                // exits naturally when the reply stream closes — i.e.,
                // when the channel is dropped on the client side.
                // `Unsubscribe` is a hint that lets clients keep the
                // channel open while no longer receiving events; today
                // it acks immediately (real teardown of an in-flight
                // forwarder requires per-channel cancellation handles,
                // tracked as a follow-up).
                ManagerResponse::Unsubscribed
            }
            ManagerRequest::Reconnect { id } => {
                let exists = self.connections.read().await.contains_key(&id);
                if !exists {
                    ManagerResponse::from(io::Error::new(
                        io::ErrorKind::NotConnected,
                        "No connection found",
                    ))
                } else {
                    info!("[Conn {id}] Manual reconnection requested");
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

    use tokio::sync::mpsc;

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
        Config::default()
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
        let (event_tx, _event_rx) = broadcast::channel::<Event>(16);

        let server = ManagerServer {
            config,
            channels: RwLock::new(HashMap::new()),
            connections: Arc::new(RwLock::new(HashMap::new())),
            registry,
            managed_tunnels: RwLock::new(HashMap::new()),
            mounts: RwLock::new(HashMap::new()),
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

    #[tokio::test]
    async fn publish_connection_state_should_broadcast_event() {
        let (event_tx, mut event_rx) = broadcast::channel::<Event>(16);
        let id: ConnectionId = 42;

        publish_connection_state(&event_tx, id, ConnectionState::Reconnecting);

        let event = event_rx.recv().await.unwrap();
        match event {
            Event::ConnectionState { id: recv_id, state } => {
                assert_eq!(recv_id, 42);
                assert_eq!(state, ConnectionState::Reconnecting);
            }
            other => panic!("Expected ConnectionState event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn publish_connection_state_should_not_panic_with_no_subscribers() {
        let (event_tx, _) = broadcast::channel::<Event>(16);
        publish_connection_state(&event_tx, 1, ConnectionState::Disconnected);
    }
}
