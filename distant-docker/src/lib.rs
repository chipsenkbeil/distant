//! Docker backend for distant, enabling container interaction through the distant API.
//!
//! This crate provides a client-side plugin that translates distant operations to Docker API
//! calls, supporting file I/O (via tar archives), process management (via exec), directory
//! operations, and search (best-effort with tool detection).
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

use bollard::Docker as BollardDocker;
use bollard::models::ContainerCreateBody;
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, InspectContainerOptions, RemoveContainerOptionsBuilder,
    StartContainerOptions, StopContainerOptionsBuilder,
};
use distant_core::net::auth::{DummyAuthHandler, Verifier};
use distant_core::net::client::{Client as NetClient, ClientConfig};
use distant_core::net::common::{InmemoryTransport, OneshotListener};
use distant_core::net::server::{Server, ServerRef};
use distant_core::{ApiServerHandler, Client};
use futures::StreamExt;
use log::*;

mod api;
mod process;
pub(crate) mod search;
pub mod utils;

use api::DockerApi;

/// Represents the OS family of the container.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum DockerFamily {
    /// Container runs a Unix-based OS (Linux, etc.)
    Unix,

    /// Container runs a Windows-based OS
    Windows,
}

impl DockerFamily {
    /// Returns the family as a static string.
    pub const fn as_static_str(&self) -> &'static str {
        match self {
            Self::Unix => "unix",
            Self::Windows => "windows",
        }
    }
}

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
            image: String::from("ubuntu:22.04"),
            auto_remove: false,
        }
    }
}

/// Represents a connection to a Docker container.
pub struct Docker {
    /// Bollard Docker client handle.
    client: BollardDocker,

    /// Name or ID of the connected container.
    container: String,

    /// Detected OS family of the container.
    family: DockerFamily,

    /// Connection options.
    opts: DockerOpts,
}

impl Docker {
    /// Connect to an existing, running Docker container by name or ID.
    ///
    /// Verifies the container is running and detects its OS family.
    ///
    /// # Errors
    ///
    /// Returns an error if the Docker daemon is unreachable, the container does not exist,
    /// or the container is not running.
    pub async fn connect(container: impl Into<String>, opts: DockerOpts) -> io::Result<Self> {
        let container = container.into();
        let client = Self::create_bollard_client(&opts)?;

        info!("Connecting to Docker container: {}", container);

        // Verify the container exists and is running
        let inspect = client
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

        let family = Self::detect_family_from_inspect(&inspect, &client, &container).await?;
        info!(
            "Connected to container '{}' (family: {})",
            container,
            family.as_static_str()
        );

        Ok(Self {
            client,
            container,
            family,
            opts,
        })
    }

    /// Launch a new container from an image and connect to it.
    ///
    /// Pulls the image if it is not available locally. The container is started with a
    /// keep-alive entrypoint (`sleep infinity` on Linux, `ping -t localhost` on Windows).
    ///
    /// # Errors
    ///
    /// Returns an error if the image cannot be pulled, the container cannot be created,
    /// or the container fails to start.
    pub async fn launch(launch_opts: LaunchOpts, docker_opts: DockerOpts) -> io::Result<Self> {
        let client = Self::create_bollard_client(&docker_opts)?;

        info!("Launching container from image: {}", launch_opts.image);

        // Pull the image if needed
        Self::pull_image_if_needed(&client, &launch_opts.image).await?;

        // Detect if this is a Windows image by inspecting the image
        let is_windows = Self::is_windows_image(&client, &launch_opts.image).await;

        let (entrypoint, cmd) = if is_windows {
            (
                Some(vec!["cmd".to_string(), "/c".to_string()]),
                Some(vec![
                    "ping".to_string(),
                    "-t".to_string(),
                    "localhost".to_string(),
                ]),
            )
        } else {
            (
                None,
                Some(vec!["sleep".to_string(), "infinity".to_string()]),
            )
        };

        let container_name = format!("distant-{}", &uuid_like_id());

        let config = ContainerCreateBody {
            image: Some(launch_opts.image.clone()),
            entrypoint,
            cmd,
            tty: Some(false),
            open_stdin: Some(true),
            ..Default::default()
        };

        let create_opts = CreateContainerOptionsBuilder::default()
            .name(&container_name)
            .build();

        let container_id = client
            .create_container(Some(create_opts), config)
            .await
            .map_err(|e| io::Error::other(format!("Failed to create container: {}", e)))?
            .id;

        debug!("Created container: {} ({})", container_name, container_id);

        // Start the container
        client
            .start_container(&container_id, None::<StartContainerOptions>)
            .await
            .map_err(|e| io::Error::other(format!("Failed to start container: {}", e)))?;

        info!("Container started: {}", container_name);

        let family = if is_windows {
            DockerFamily::Windows
        } else {
            DockerFamily::Unix
        };

        Ok(Self {
            client,
            container: container_name,
            family,
            opts: docker_opts,
        })
    }

    /// Converts this Docker connection into a distant [`Client`].
    ///
    /// Creates an in-memory server/client pair where the server side is backed by [`DockerApi`].
    pub async fn into_distant_client(self) -> io::Result<Client> {
        let api = DockerApi::new(self.client, self.container, self.family, self.opts).await;

        let (t1, t2) = InmemoryTransport::pair(100);

        let server = Server::new()
            .handler(ApiServerHandler::new(api))
            .verifier(Verifier::none());

        tokio::spawn(async move {
            let _ = server.start(OneshotListener::from_value(t2));
        });

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
    pub async fn into_distant_pair(self) -> io::Result<(Client, ServerRef)> {
        let api = DockerApi::new(self.client, self.container, self.family, self.opts).await;

        let (t1, t2) = InmemoryTransport::pair(100);

        let server = Server::new()
            .handler(ApiServerHandler::new(api))
            .verifier(Verifier::none());

        let server_ref = server
            .start(OneshotListener::from_value(t2))
            .map_err(io::Error::other)?;

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

    /// Returns the detected OS family.
    pub fn family(&self) -> DockerFamily {
        self.family
    }

    /// Creates a bollard Docker client from the provided options.
    fn create_bollard_client(opts: &DockerOpts) -> io::Result<BollardDocker> {
        match &opts.docker_host {
            Some(host) => {
                BollardDocker::connect_with_socket(host, 120, bollard::API_DEFAULT_VERSION).map_err(
                    |e| {
                        io::Error::new(
                            io::ErrorKind::ConnectionRefused,
                            format!("Failed to connect to Docker daemon at '{}': {}", host, e),
                        )
                    },
                )
            }
            None => Self::default_bollard_client(),
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

    /// Detects the container's OS family from inspection data, with exec fallback.
    async fn detect_family_from_inspect(
        inspect: &bollard::models::ContainerInspectResponse,
        client: &BollardDocker,
        container: &str,
    ) -> io::Result<DockerFamily> {
        // Check platform from container config labels
        if let Some(platform) = inspect
            .config
            .as_ref()
            .and_then(|c| c.labels.as_ref())
            .and_then(|l| l.get("org.opencontainers.image.os"))
        {
            if platform.to_lowercase().contains("windows") {
                return Ok(DockerFamily::Windows);
            }
            return Ok(DockerFamily::Unix);
        }

        // Fallback: try running uname via exec
        match utils::execute_output(client, container, &["uname", "-s"], None).await {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let lower = stdout.trim().to_lowercase();
                if lower.contains("windows") || lower.contains("mingw") || lower.contains("msys") {
                    Ok(DockerFamily::Windows)
                } else {
                    Ok(DockerFamily::Unix)
                }
            }
            Err(_) => {
                // If uname fails, try `cmd /c ver` for Windows detection
                match utils::execute_output(client, container, &["cmd", "/c", "ver"], None).await {
                    Ok(output) if output.stdout_str().to_lowercase().contains("windows") => {
                        Ok(DockerFamily::Windows)
                    }
                    _ => {
                        debug!("Could not determine container OS family, defaulting to Unix");
                        Ok(DockerFamily::Unix)
                    }
                }
            }
        }
    }

    /// Pulls an image if it doesn't exist locally.
    async fn pull_image_if_needed(client: &BollardDocker, image: &str) -> io::Result<()> {
        if client.inspect_image(image).await.is_ok() {
            debug!("Image '{}' already exists locally", image);
            return Ok(());
        }

        info!("Pulling image '{}'...", image);

        use bollard::query_parameters::CreateImageOptionsBuilder;
        let options = CreateImageOptionsBuilder::default()
            .from_image(image)
            .build();

        let mut stream = client.create_image(Some(options), None, None);

        while let Some(result) = stream.next().await {
            match result {
                Ok(info) => {
                    if let Some(status) = &info.status {
                        trace!("Pull progress: {}", status);
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

    /// Checks if an image is Windows-based by inspecting its OS field.
    async fn is_windows_image(client: &BollardDocker, image: &str) -> bool {
        match client.inspect_image(image).await {
            Ok(info) => info
                .os
                .as_ref()
                .is_some_and(|os| os.to_lowercase() == "windows"),
            Err(_) => false,
        }
    }

    /// Stops and removes the container. Used for auto-remove cleanup.
    pub async fn stop_and_remove(client: &BollardDocker, container: &str) -> io::Result<()> {
        debug!("Stopping container '{}'", container);
        let stop_opts = StopContainerOptionsBuilder::default().t(5).build();
        let _ = client.stop_container(container, Some(stop_opts)).await;

        debug!("Removing container '{}'", container);
        let remove_opts = RemoveContainerOptionsBuilder::default().force(true).build();
        client
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
