//! macOS FileProvider extension entry point.
//!
//! When the binary is launched inside a `.appex` bundle, `is_file_provider_extension()`
//! returns `true` and `run_extension()` takes over before the CLI parser runs.
//! The extension connects to the distant manager via the App Group shared container
//! and blocks to serve FileProvider requests.

use std::time::Duration;

use distant_core::auth::DummyAuthHandler;
use distant_core::net::client::{Client as NetClient, ClientConfig, ReconnectStrategy};
use distant_core::net::common::ConnectionId;
use distant_core::net::manager::PROTOCOL_VERSION;

use crate::cli::logger;
use crate::constants;

/// Returns `true` if this process is running as a `.appex` FileProvider extension.
///
/// Delegates to `distant_mount::is_file_provider_extension()` which uses
/// `NSBundle.mainBundle.bundlePath` to check for a `.appex` suffix.
pub fn is_file_provider_extension() -> bool {
    distant_mount::is_file_provider_extension()
}

/// Runs the FileProvider extension process.
///
/// Creates a Tokio runtime, initialises file-based logging, and registers the
/// channel resolver with `distant_mount` so the `.appex` can bootstrap a
/// [`RemoteFs`] when macOS calls `initWithDomain:`.
///
/// Blocks forever. macOS `fileproviderd` will instantiate the
/// `DistantFileProvider` class (defined in `distant-mount`) via
/// `initWithDomain:` on a background XPC queue, which triggers the
/// bootstrap flow.
pub fn run_extension() -> ! {
    init_appex_logging();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("failed to create tokio runtime for .appex");

    let handle = rt.handle().clone();
    distant_mount::init_file_provider(
        rt.handle().clone(),
        Box::new(move |connection_id, destination| {
            handle.block_on(resolve_connection(connection_id, destination))
        }),
    );

    // Block the main thread forever. The FileProvider framework communicates
    // via XPC on background dispatch queues, so the main thread just needs
    // to stay alive.
    loop {
        std::thread::park();
    }
}

/// Resolves a stored connection ID and destination into a live
/// [`distant_core::Channel`] by communicating with the distant manager.
///
/// Tries the stored `connection_id` first (fast path). If it is no longer
/// valid, falls back to searching the manager's connection list by
/// destination string.
async fn resolve_connection(
    connection_id: u32,
    destination: &str,
) -> std::io::Result<distant_core::Channel> {
    let mut client = connect_headless().await?;

    // Fast path: try the stored connection_id
    let resolved_id: ConnectionId = match client.info(connection_id).await {
        Ok(info) if info.destination == destination => connection_id,
        _ => {
            log::debug!("appex: connection {connection_id} gone, searching by destination");
            let list = client.list().await?;
            list.iter()
                .find(|(_, dest)| dest.as_str() == destination)
                .map(|(id, _)| *id)
                .ok_or_else(|| {
                    std::io::Error::other(format!(
                        "no connection for {destination}. \
                         Run `distant connect {destination}` to re-establish."
                    ))
                })?
        }
    };

    let raw = client.open_raw_channel(resolved_id).await?;
    Ok(raw.into_client().into_channel())
}

/// Connects to the distant manager daemon over the App Group shared socket
/// using [`DummyAuthHandler`] (no interactive auth in the `.appex`).
async fn connect_headless() -> std::io::Result<distant_core::net::manager::ManagerClient> {
    let socket_path = &*constants::user::UNIX_SOCKET_PATH;

    NetClient::unix_socket(socket_path)
        .auth_handler(DummyAuthHandler)
        .config(ClientConfig {
            reconnect_strategy: ReconnectStrategy::ExponentialBackoff {
                base: Duration::from_millis(200),
                factor: 2.0,
                max_duration: Some(Duration::from_secs(2)),
                max_retries: Some(5),
                timeout: None,
            },
            ..Default::default()
        })
        .version(PROTOCOL_VERSION)
        .connect()
        .await
        .map_err(|e| {
            std::io::Error::other(format!(
                "failed to connect to distant manager at {}: {}. \
                 Ensure `distant manager listen --daemon` is running.",
                socket_path.display(),
                e
            ))
        })
}

/// Initialises file-based logging for the `.appex` extension process.
///
/// Writes to `distant-appex.log` in the same directory as the manager socket
/// (the App Group shared container on macOS).
fn init_appex_logging() {
    let log_dir = constants::user::UNIX_SOCKET_PATH
        .parent()
        .expect("socket path has parent");
    let _ = std::fs::create_dir_all(log_dir);

    logger::FileLogger::init(
        vec![
            "distant".to_string(),
            "distant_core".to_string(),
            "distant_mount".to_string(),
        ],
        log::LevelFilter::Debug,
        &log_dir.join("distant-appex.log"),
    );
}
