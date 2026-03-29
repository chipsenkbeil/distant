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
pub fn is_appex() -> bool {
    distant_mount::macos::is_file_provider_extension()
}

/// Runs the FileProvider extension process.
///
/// Creates a Tokio runtime, initialises file-based logging, and registers the
/// channel resolver with `distant_mount` so the `.appex` can bootstrap a
/// [`RemoteFs`] when macOS calls `initWithDomain:`.
///
/// Calls `_NSExtensionMain` which sets up the PlugInKit XPC listener that
/// reads `NSExtensionPrincipalClass` from `Info.plist` and calls
/// `initWithDomain:` when `fileproviderd` connects.
pub fn main() -> ! {
    init_appex_logging();

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("failed to create tokio runtime for .appex");

    let handle = rt.handle().clone();
    distant_mount::macos::init_file_provider(
        rt.handle().clone(),
        Box::new(move |connection_id, destination| {
            handle.block_on(resolve_connection(connection_id, destination))
        }),
    );

    // NSExtensionMain (exported as _NSExtensionMain from Foundation) sets up
    // the PlugInKit XPC listener that reads NSExtensionPrincipalClass from
    // Info.plist and calls initWithDomain: when fileproviderd connects.
    //
    // We must pass the real argc/argv from the process; passing (0, null)
    // causes a SIGSEGV inside Foundation's argument parsing.
    unsafe extern "C" {
        fn _NSGetArgc() -> *mut i32;
        fn _NSGetArgv() -> *mut *mut *mut i8;
        fn NSExtensionMain(argc: i32, argv: *const *const i8) -> i32;
    }
    unsafe {
        let argc = *_NSGetArgc();
        let argv = *_NSGetArgv() as *const *const i8;
        NSExtensionMain(argc, argv);
    }
    std::process::exit(0);
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
    log::info!("appex: resolving channel for connection_id={connection_id}, dest={destination:?}");
    let mut client = connect_headless().await?;
    log::info!("appex: connected to manager daemon");

    // Fast path: try the stored connection_id
    let resolved_id: ConnectionId = match client.info(connection_id).await {
        Ok(info) if info.destination == destination => {
            log::info!("appex: connection {connection_id} still valid (fast path)");
            connection_id
        }
        Ok(info) => {
            log::debug!(
                "appex: connection {connection_id} destination mismatch (got {:?}), searching",
                info.destination
            );
            find_connection_by_destination(&mut client, destination).await?
        }
        Err(e) => {
            log::debug!(
                "appex: connection {connection_id} lookup failed ({e}), searching by destination"
            );
            find_connection_by_destination(&mut client, destination).await?
        }
    };

    log::info!("appex: opening raw channel for connection {resolved_id}");
    let raw = client.open_raw_channel(resolved_id).await?;
    log::info!("appex: channel opened successfully");
    Ok(raw.into_client().into_channel())
}

/// Searches the manager's connection list for a connection matching `destination`.
async fn find_connection_by_destination(
    client: &mut distant_core::net::manager::ManagerClient,
    destination: &str,
) -> std::io::Result<ConnectionId> {
    let list = client.list().await?;
    log::debug!("appex: manager has {} active connections", list.len());
    list.iter()
        .find(|(_, dest)| dest.as_str() == destination)
        .map(|(id, _)| *id)
        .ok_or_else(|| {
            std::io::Error::other(format!(
                "no connection for {destination}. \
                 Run `distant connect {destination}` to re-establish."
            ))
        })
}

/// Connects to the distant manager daemon over the App Group shared socket
/// using [`DummyAuthHandler`] (no interactive auth in the `.appex`).
///
/// Uses `NSFileManager.containerURL` to resolve the real group container
/// path, which works correctly from inside the sandbox.
async fn connect_headless() -> std::io::Result<distant_core::net::manager::ManagerClient> {
    let socket_path = distant_mount::macos::app_group_container_path()
        .map(|p| p.join("distant.sock"))
        .unwrap_or_else(|| constants::user::UNIX_SOCKET_PATH.clone());

    NetClient::unix_socket(&socket_path)
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

/// Initialises logging for the `.appex` extension process.
///
/// Tries `logs/distant-appex-{pid}.log` in the App Group shared container
/// first; falls back to `/tmp/distant-appex-{pid}.log` if the container
/// is unavailable. Always enables stderr as a secondary output.
///
/// Defaults to `Trace` level so that the bootstrap flow logs everything;
/// `bootstrap()` may later tighten the level via `log::set_max_level()`
/// if the domain metadata contains a `log_level` key.
fn init_appex_logging() {
    let pid = std::process::id();
    let log_filename = format!("distant-appex-{pid}.log");

    let log_path = distant_mount::macos::app_group_container_path()
        .and_then(|container| {
            let log_dir = container.join("logs");
            std::fs::create_dir_all(&log_dir).ok()?;
            Some(log_dir.join(&log_filename))
        })
        .unwrap_or_else(|| std::env::temp_dir().join(&log_filename));

    let modules = vec![
        "distant".to_string(),
        "distant_core".to_string(),
        "distant_mount".to_string(),
    ];

    let _ = logger::Logger::builder()
        .modules(modules)
        .level(log::LevelFilter::Trace)
        .file(&log_path)
        .stderr()
        .init();
}
