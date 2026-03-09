//! Docker backend for distant, enabling container interaction through the distant API.
//!
//! This crate provides a client-side plugin that translates distant operations to Docker API
//! calls, supporting file I/O (via tar archives), process management (via exec), directory
//! operations, and search (best-effort with tool detection).
//!
//! Only Unix containers are supported. The Docker host can be any platform (Linux, macOS,
//! Windows), but the container must run a Unix-based OS.
//!
//! # Usage
//!
//! ```no_run
//! use distant_docker::{Docker, DockerOpts};
//!
//! # async fn example() -> std::io::Result<()> {
//! // Connect to an existing container
//! let docker = Docker::connect("my-container", DockerOpts::default()).await?;
//! let client = docker.into_distant_client().await?;
//! # Ok(())
//! # }
//! ```

#![allow(clippy::manual_async_fn)]

use std::io;
use std::time::Duration;

use bollard::Docker as BollardDocker;
use bollard::models::ContainerCreateBody;
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, InspectContainerOptions, RemoveContainerOptionsBuilder,
    StartContainerOptions, StopContainerOptionsBuilder,
};
use distant_core::net::auth::{DummyAuthHandler, Verifier};
use distant_core::net::client::{Client as NetClient, ClientConfig};
use distant_core::net::common::{InmemoryTransport, OneshotListener};
use distant_core::net::server::{Server, ServerRef, ShutdownSender};
use distant_core::{ApiServerHandler, Client};
use futures::StreamExt;
use log::*;

/// Thin wrapper around the bollard Docker client.
///
/// Provides discovery-based connection to the Docker daemon with platform-specific socket
/// detection. This type encapsulates the bollard dependency so that downstream code does not
/// depend on it directly.
#[derive(Debug, Clone)]
pub struct DockerClient(BollardDocker);

impl DockerClient {
    /// Connect to the Docker daemon using automatic discovery.
    ///
    /// Tries the following locations in order:
    ///
    /// 1. `DOCKER_HOST` env var and the platform default socket (via bollard's built-in logic)
    /// 2. Docker Desktop socket at `~/.docker/run/docker.sock` (macOS / Linux)
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::NotFound`] if no reachable Docker socket is found.
    pub fn connect_default() -> io::Result<Self> {
        Docker::default_bollard_client().map(Self)
    }

    /// Connect to the Docker daemon at a specific socket URI.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection cannot be established.
    pub fn connect_with_socket(uri: &str) -> io::Result<Self> {
        BollardDocker::connect_with_socket(uri, 120, bollard::API_DEFAULT_VERSION)
            .map(Self)
            .map_err(|e| {
                io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    format!("Failed to connect to Docker daemon at '{}': {}", uri, e),
                )
            })
    }

    /// Ping the Docker daemon to check connectivity.
    ///
    /// # Errors
    ///
    /// Returns an error if the daemon does not respond.
    pub async fn ping(&self) -> io::Result<()> {
        self.0
            .ping()
            .await
            .map(|_| ())
            .map_err(|e| io::Error::other(format!("Docker ping failed: {}", e)))
    }

    /// Check whether a Docker image is available locally.
    pub async fn has_image(&self, image: &str) -> bool {
        self.0.inspect_image(image).await.is_ok()
    }

    /// Pull a Docker image.
    ///
    /// # Errors
    ///
    /// Returns an error if the pull fails.
    pub async fn pull_image(&self, image: &str) -> io::Result<()> {
        use bollard::query_parameters::CreateImageOptionsBuilder;

        let options = CreateImageOptionsBuilder::default()
            .from_image(image)
            .build();
        let mut stream = self.0.create_image(Some(options), None, None);
        while let Some(result) = stream.next().await {
            result.map_err(|e| {
                io::Error::other(format!("Failed to pull image '{}': {}", image, e))
            })?;
        }
        Ok(())
    }

    /// Create and start a container, returning its ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the container cannot be created or started.
    pub async fn create_and_start_container(
        &self,
        name: &str,
        image: &str,
        cmd: Vec<String>,
    ) -> io::Result<String> {
        use bollard::models::ContainerCreateBody;
        use bollard::query_parameters::{CreateContainerOptionsBuilder, StartContainerOptions};

        let config = ContainerCreateBody {
            image: Some(image.to_string()),
            cmd: Some(cmd),
            tty: Some(false),
            open_stdin: Some(true),
            ..Default::default()
        };

        let create_opts = CreateContainerOptionsBuilder::default().name(name).build();

        let response = self
            .0
            .create_container(Some(create_opts), config)
            .await
            .map_err(|e| {
                io::Error::other(format!("Failed to create container '{}': {}", name, e))
            })?;

        self.0
            .start_container(&response.id, None::<StartContainerOptions>)
            .await
            .map_err(|e| {
                io::Error::other(format!("Failed to start container '{}': {}", name, e))
            })?;

        Ok(response.id)
    }

    /// Check whether the Docker daemon is running a Linux engine.
    ///
    /// Returns `true` when the daemon reports `OSType` as anything other than `"windows"`,
    /// or when the info call succeeds but `os_type` is absent (assumed Linux).
    /// Returns `false` if the daemon reports Windows containers or if the info call fails.
    ///
    /// This is useful for gating tests that require Linux containers — the `distant-docker`
    /// crate only supports Unix containers.
    pub async fn is_linux_engine(&self) -> bool {
        match self.0.info().await {
            Ok(info) => info.os_type.as_deref() != Some("windows"),
            Err(_) => false,
        }
    }

    /// Returns a reference to the inner bollard client.
    pub(crate) fn inner(&self) -> &BollardDocker {
        &self.0
    }
}

mod api;
mod plugin;
mod process;
pub(crate) mod search;
pub mod utils;

pub use plugin::DockerPlugin;

use api::DockerApi;

/// Options for connecting to or launching Docker containers.
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct DockerOpts {
    /// Optional Docker daemon URI (e.g., `unix:///var/run/docker.sock`).
    /// If not provided, uses the default local connection.
    pub docker_host: Option<String>,

    /// User to run exec commands as (overrides container default).
    pub user: Option<String>,

    /// Default working directory for operations.
    pub working_dir: Option<String>,

    /// Shell override (defaults to auto-detection based on container OS).
    pub shell: Option<String>,
}

/// Options for launching a new container from an image.
#[derive(Clone, Debug)]
pub struct LaunchOpts {
    /// Docker image to use (e.g., `ubuntu:22.04`).
    pub image: String,

    /// If true, the container is stopped and removed on disconnect.
    pub auto_remove: bool,
}

impl Default for LaunchOpts {
    fn default() -> Self {
        Self {
            image: String::new(),
            auto_remove: true,
        }
    }
}

/// Represents a connection to a Docker container.
pub struct Docker {
    /// Docker client handle.
    client: DockerClient,

    /// Name or ID of the connected container.
    container: String,

    /// Connection options.
    opts: DockerOpts,

    /// If true, stop and remove the container when the server shuts down.
    auto_remove: bool,
}

impl Docker {
    /// Connect to an existing, running Docker container by name or ID.
    ///
    /// Verifies the container is running before returning.
    ///
    /// # Errors
    ///
    /// Returns an error if the Docker daemon is unreachable, the container does not exist,
    /// or the container is not running.
    pub async fn connect(container: impl Into<String>, opts: DockerOpts) -> io::Result<Self> {
        let container = container.into();
        let client = Self::create_client(&opts)?;

        info!("Connecting to Docker container: {}", container);

        // Verify the container exists and is running
        let inspect = client
            .inner()
            .inspect_container(&container, None::<InspectContainerOptions>)
            .await
            .map_err(|e| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("Failed to inspect container '{}': {}", container, e),
                )
            })?;

        let running = inspect
            .state
            .as_ref()
            .and_then(|s| s.running)
            .unwrap_or(false);

        if !running {
            return Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                format!("Container '{}' is not running", container),
            ));
        }

        info!("Connected to container '{}'", container);

        Ok(Self {
            client,
            container,
            opts,
            auto_remove: false,
        })
    }

    /// Launch a new container from an image and connect to it.
    ///
    /// Pulls the image if it is not available locally. The container is started with a
    /// keep-alive entrypoint (`sleep infinity`).
    ///
    /// # Errors
    ///
    /// Returns an error if the image cannot be pulled, the container cannot be created,
    /// or the container fails to start.
    pub async fn launch(launch_opts: LaunchOpts, docker_opts: DockerOpts) -> io::Result<Self> {
        if launch_opts.image.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Docker image must be specified (e.g. 'ubuntu:22.04')",
            ));
        }

        let client = Self::create_client(&docker_opts)?;

        info!("Launching container from image: {}", launch_opts.image);

        // Pull the image if needed
        Self::pull_image_if_needed(&client, &launch_opts.image).await?;

        let container_name = format!("distant-{}", &uuid_like_id());

        let config = ContainerCreateBody {
            image: Some(launch_opts.image.clone()),
            cmd: Some(vec!["sleep".to_string(), "infinity".to_string()]),
            tty: Some(false),
            open_stdin: Some(true),
            ..Default::default()
        };

        let create_opts = CreateContainerOptionsBuilder::default()
            .name(&container_name)
            .build();

        let container_id = client
            .inner()
            .create_container(Some(create_opts), config)
            .await
            .map_err(|e| io::Error::other(format!("Failed to create container: {}", e)))?
            .id;

        debug!("Created container: {} ({})", container_name, container_id);

        // Start the container
        client
            .inner()
            .start_container(&container_id, None::<StartContainerOptions>)
            .await
            .map_err(|e| io::Error::other(format!("Failed to start container: {}", e)))?;

        info!("Container started: {}", container_name);

        Ok(Self {
            client,
            container: container_name,
            opts: docker_opts,
            auto_remove: launch_opts.auto_remove,
        })
    }

    /// Converts this Docker connection into a distant [`Client`].
    ///
    /// Creates an in-memory server/client pair where the server side is backed by [`DockerApi`].
    /// If `auto_remove` is enabled, the container is stopped and removed when the server shuts
    /// down. A health monitor task is spawned to detect Docker daemon or container death and
    /// trigger server shutdown, which causes the client to see a disconnect.
    pub async fn into_distant_client(self) -> io::Result<Client> {
        let auto_remove = self.auto_remove;
        let cleanup_client = if auto_remove {
            Some(Self::create_client(&self.opts)?)
        } else {
            None
        };
        let container_name = self.container.clone();
        let health_client = self.client.clone();
        let health_container = self.container.clone();

        let api = DockerApi::new(self.client.clone(), self.container, self.opts).await;

        let (t1, t2) = InmemoryTransport::pair(100);

        let server = Server::new()
            .handler(ApiServerHandler::new(api))
            .verifier(Verifier::none());

        let server_ref = server
            .start(OneshotListener::from_value(t2))
            .map_err(io::Error::other)?;

        // Spawn cleanup task if auto_remove is set
        if let Some(client) = cleanup_client {
            let mut shutdown_rx = server_ref.subscribe_shutdown();
            let cleanup_container = container_name.clone();
            tokio::spawn(async move {
                let _ = shutdown_rx.recv().await;
                info!(
                    "Auto-removing container '{}' after server shutdown",
                    cleanup_container
                );
                if let Err(e) = Self::stop_and_remove(&client, &cleanup_container).await {
                    warn!(
                        "Failed to auto-remove container '{}': {}",
                        cleanup_container, e
                    );
                }
            });
        }

        // Spawn health monitor that detects Docker daemon/container death
        tokio::spawn(Self::docker_health_monitor(
            health_client,
            health_container,
            server_ref.shutdown_sender(),
        ));

        let client = NetClient::build()
            .auth_handler(DummyAuthHandler)
            .config(ClientConfig::default())
            .connector(t1)
            .connect()
            .await
            .map_err(io::Error::other)?;

        Ok(client)
    }

    /// Converts this Docker connection into a pair of distant client and server ref.
    ///
    /// If `auto_remove` is enabled, the container is stopped and removed when the server
    /// shuts down. A health monitor task is spawned to detect Docker daemon or container
    /// death and trigger server shutdown.
    pub async fn into_distant_pair(self) -> io::Result<(Client, ServerRef)> {
        let auto_remove = self.auto_remove;
        let cleanup_client = if auto_remove {
            Some(Self::create_client(&self.opts)?)
        } else {
            None
        };
        let container_name = self.container.clone();
        let health_client = self.client.clone();
        let health_container = self.container.clone();

        let api = DockerApi::new(self.client, self.container, self.opts).await;

        let (t1, t2) = InmemoryTransport::pair(100);

        let server = Server::new()
            .handler(ApiServerHandler::new(api))
            .verifier(Verifier::none());

        let server_ref = server
            .start(OneshotListener::from_value(t2))
            .map_err(io::Error::other)?;

        // Spawn cleanup task that waits for server shutdown signal
        if let Some(client) = cleanup_client {
            let mut shutdown_rx = server_ref.subscribe_shutdown();
            let cleanup_container = container_name;
            tokio::spawn(async move {
                let _ = shutdown_rx.recv().await;
                info!(
                    "Auto-removing container '{}' after server shutdown",
                    cleanup_container
                );
                if let Err(e) = Self::stop_and_remove(&client, &cleanup_container).await {
                    warn!(
                        "Failed to auto-remove container '{}': {}",
                        cleanup_container, e
                    );
                }
            });
        }

        // Spawn health monitor that detects Docker daemon/container death
        tokio::spawn(Self::docker_health_monitor(
            health_client,
            health_container,
            server_ref.shutdown_sender(),
        ));

        let client = NetClient::build()
            .auth_handler(DummyAuthHandler)
            .config(ClientConfig::default())
            .connector(t1)
            .connect()
            .await
            .map_err(io::Error::other)?;

        Ok((client, server_ref))
    }

    /// Returns the container name or ID.
    pub fn container(&self) -> &str {
        &self.container
    }

    /// Monitors Docker daemon and container health, triggering server shutdown on failure.
    ///
    /// Checks every 5 seconds:
    /// 1. Docker daemon responsiveness via ping
    /// 2. Container running state via inspect
    ///
    /// When either check fails, the server shutdown signal is sent, which drops
    /// the in-memory transport and causes the client to see a disconnect.
    async fn docker_health_monitor(
        client: DockerClient,
        container: String,
        shutdown: ShutdownSender,
    ) {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;

            // Check daemon responsiveness
            if client.ping().await.is_err() {
                warn!(
                    "Docker daemon unreachable, triggering server shutdown for container '{container}'"
                );
                shutdown.shutdown();
                return;
            }

            // Check container state
            match client
                .inner()
                .inspect_container(&container, None::<InspectContainerOptions>)
                .await
            {
                Ok(info) => {
                    let running = info.state.as_ref().and_then(|s| s.running).unwrap_or(false);
                    if !running {
                        warn!(
                            "Container '{container}' is no longer running, triggering server shutdown"
                        );
                        shutdown.shutdown();
                        return;
                    }
                }
                Err(e) => {
                    warn!(
                        "Cannot inspect container '{container}': {e}, triggering server shutdown"
                    );
                    shutdown.shutdown();
                    return;
                }
            }
        }
    }

    /// Creates a Docker client from the provided options.
    fn create_client(opts: &DockerOpts) -> io::Result<DockerClient> {
        match &opts.docker_host {
            Some(host) => DockerClient::connect_with_socket(host),
            None => DockerClient::connect_default(),
        }
    }

    /// Connects to the Docker daemon using automatic discovery.
    ///
    /// Tries the following locations in order:
    ///
    /// 1. `DOCKER_HOST` env var and the platform default socket (via bollard's built-in logic)
    /// 2. Docker Desktop socket at `~/.docker/run/docker.sock` (macOS / Linux)
    ///
    /// This is exposed publicly so that test harnesses can reuse the same discovery logic.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::NotFound`] if no reachable Docker socket is found.
    pub fn default_bollard_client() -> io::Result<BollardDocker> {
        // Try bollard's built-in default (checks DOCKER_HOST, then /var/run/docker.sock or
        // the Windows named pipe)
        if let Ok(client) = BollardDocker::connect_with_local_defaults() {
            return Ok(client);
        }

        // On Unix, try the Docker Desktop socket under the user's home directory
        #[cfg(unix)]
        if let Some(home) = std::env::var_os("HOME") {
            let desktop_sock = std::path::Path::new(&home).join(".docker/run/docker.sock");
            if desktop_sock.exists() {
                let uri = format!("unix://{}", desktop_sock.display());
                return BollardDocker::connect_with_unix(&uri, 120, bollard::API_DEFAULT_VERSION)
                    .map_err(|e| {
                        io::Error::new(
                            io::ErrorKind::ConnectionRefused,
                            format!("Failed to connect to Docker Desktop socket: {e}"),
                        )
                    });
            }
        }

        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Docker daemon not found. Set DOCKER_HOST or ensure Docker is running.",
        ))
    }

    /// Pulls an image if it doesn't exist locally.
    async fn pull_image_if_needed(client: &DockerClient, image: &str) -> io::Result<()> {
        if client.inner().inspect_image(image).await.is_ok() {
            debug!("Image '{}' already exists locally", image);
            return Ok(());
        }

        info!("Pulling image '{}'...", image);

        use bollard::query_parameters::CreateImageOptionsBuilder;
        let options = CreateImageOptionsBuilder::default()
            .from_image(image)
            .build();

        let mut stream = client.inner().create_image(Some(options), None, None);

        while let Some(result) = stream.next().await {
            match result {
                Ok(info) => {
                    if let Some(status) = &info.status {
                        if let Some(detail) = &info.progress_detail {
                            match (detail.current, detail.total) {
                                (Some(current), Some(total)) => {
                                    info!("Pull: {} ({}/{})", status, current, total);
                                }
                                _ => {
                                    info!("Pull: {}", status);
                                }
                            }
                        } else {
                            info!("Pull: {}", status);
                        }
                    }
                }
                Err(e) => {
                    return Err(io::Error::other(format!(
                        "Failed to pull image '{}': {}",
                        image, e
                    )));
                }
            }
        }

        info!("Image '{}' pulled successfully", image);
        Ok(())
    }

    /// Stops and removes the container. Used for auto-remove cleanup.
    pub async fn stop_and_remove(client: &DockerClient, container: &str) -> io::Result<()> {
        debug!("Stopping container '{}'", container);
        let stop_opts = StopContainerOptionsBuilder::default().t(5).build();
        let _ = client
            .inner()
            .stop_container(container, Some(stop_opts))
            .await;

        debug!("Removing container '{}'", container);
        let remove_opts = RemoveContainerOptionsBuilder::default().force(true).build();
        client
            .inner()
            .remove_container(container, Some(remove_opts))
            .await
            .map_err(|e| {
                io::Error::other(format!("Failed to remove container '{}': {}", container, e))
            })
    }
}

/// Generates a short random ID for container naming.
fn uuid_like_id() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 8] = rng.r#gen();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}
