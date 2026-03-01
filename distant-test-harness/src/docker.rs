//! Docker container test harness for integration tests.
//!
//! Provides [`DockerContainer`] for managing test container lifecycles and rstest fixtures
//! for obtaining [`Client`] instances connected to Docker containers.

use derive_more::{Deref, DerefMut};
use distant_core::Client;
use distant_docker::{Docker, DockerOpts};
use log::*;
use rstest::*;

/// Default test image per platform.
fn test_image() -> &'static str {
    if cfg!(windows) {
        "mcr.microsoft.com/windows/nanoserver:ltsc2025"
    } else {
        "ubuntu:22.04"
    }
}

/// Checks whether the Docker daemon is available by pinging it.
pub async fn docker_available() -> bool {
    match Docker::default_bollard_client() {
        Ok(client) => client.ping().await.is_ok(),
        Err(_) => false,
    }
}

/// A managed Docker container for testing.
///
/// Creates a container from the test image on construction and removes it on drop.
pub struct DockerContainer {
    /// The container name.
    pub name: String,

    /// Bollard client handle (kept alive for cleanup).
    client: bollard::Docker,
}

impl DockerContainer {
    /// Creates and starts a new test container.
    ///
    /// Returns `None` if Docker is unavailable (allowing tests to skip gracefully).
    pub async fn new() -> Option<Self> {
        if !docker_available().await {
            info!("Docker not available, skipping");
            return None;
        }

        let client = Docker::default_bollard_client().ok()?;

        let image = test_image();
        info!("Creating test container from image: {}", image);

        // Pull image if needed
        if client.inspect_image(image).await.is_err() {
            use bollard::query_parameters::CreateImageOptionsBuilder;
            use futures::StreamExt;

            let options = CreateImageOptionsBuilder::default()
                .from_image(image)
                .build();
            let mut stream = client.create_image(Some(options), None, None);
            while let Some(result) = stream.next().await {
                if let Err(e) = result {
                    error!("Failed to pull image: {}", e);
                    return None;
                }
            }
        }

        let name = format!("distant-test-{}", random_suffix());

        let (entrypoint, cmd) = if cfg!(windows) {
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

        use bollard::models::ContainerCreateBody;
        use bollard::query_parameters::{CreateContainerOptionsBuilder, StartContainerOptions};

        let config = ContainerCreateBody {
            image: Some(image.to_string()),
            entrypoint,
            cmd,
            tty: Some(false),
            open_stdin: Some(true),
            ..Default::default()
        };

        let create_opts = CreateContainerOptionsBuilder::default().name(&name).build();

        if let Err(e) = client.create_container(Some(create_opts), config).await {
            error!("Failed to create test container: {}", e);
            return None;
        }

        if let Err(e) = client
            .start_container(&name, None::<StartContainerOptions>)
            .await
        {
            error!("Failed to start test container: {}", e);
            // Try cleanup
            let _ = Docker::stop_and_remove(&client, &name).await;
            return None;
        }

        info!("Test container started: {}", name);

        // On Windows nanoserver, ContainerUser cannot write to C:\Windows\Temp.
        // Create a writable temp directory via the tar API (which bypasses
        // container filesystem permissions).
        if cfg!(windows)
            && let Err(e) = distant_docker::utils::tar_create_dir(&client, &name, r"C:\temp").await
        {
            error!("Failed to create temp dir in container: {}", e);
        }

        Some(Self { name, client })
    }
}

impl Drop for DockerContainer {
    fn drop(&mut self) {
        let client = self.client.clone();
        let name = self.name.clone();

        // Use a blocking approach to ensure cleanup happens
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async {
                info!("Cleaning up test container: {}", name);
                if let Err(e) = Docker::stop_and_remove(&client, &name).await {
                    error!("Failed to clean up test container '{}': {}", name, e);
                }
            });
        })
        .join()
        .ok();
    }
}

/// Wrapper holding both a value and the container that keeps it alive.
#[derive(Deref, DerefMut)]
pub struct Ctx<T> {
    /// The wrapped value (Client, Docker, etc.).
    #[deref]
    #[deref_mut]
    pub value: T,

    /// The container (kept alive while tests run).
    #[allow(dead_code)]
    pub container: DockerContainer,
}

/// Convenience macro for tests that depend on Docker.
///
/// Call at the beginning of a test body with an `Option<T>` value. If `None`, the test
/// prints a skip message and returns successfully instead of panicking.
///
/// # Examples
///
/// ```ignore
/// let container = skip_if_no_docker!(docker_container.await);
/// ```
#[macro_export]
macro_rules! skip_if_no_docker {
    ($expr:expr) => {
        match $expr {
            Some(val) => val,
            None => {
                eprintln!("Docker not available — skipping test");
                return;
            }
        }
    };
}

/// rstest fixture that provides an [`Option<DockerContainer>`].
///
/// Returns `None` if Docker is not available, allowing tests to skip gracefully
/// via [`skip_if_no_docker!`].
#[fixture]
pub async fn docker_container() -> Option<DockerContainer> {
    DockerContainer::new().await
}

/// rstest fixture that provides an [`Option<Ctx<Client>>`] connected to a Docker container.
///
/// Returns `None` if Docker is not available.
#[fixture]
pub async fn client(#[future] docker_container: Option<DockerContainer>) -> Option<Ctx<Client>> {
    let container = docker_container.await?;
    let docker = Docker::connect(&container.name, DockerOpts::default())
        .await
        .ok()?;
    let client = docker.into_distant_client().await.ok()?;
    Some(Ctx {
        value: client,
        container,
    })
}

/// Generate a short random suffix for container names.
fn random_suffix() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let bytes: [u8; 4] = rng.r#gen();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}
