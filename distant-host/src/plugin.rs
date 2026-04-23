use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::time::Duration;

use distant_core::Plugin;
use distant_core::auth::{
    AuthHandler, Authenticator, DynAuthHandler, ProxyAuthHandler, SingleAuthHandler,
    StaticKeyAuthMethodHandler,
};
use distant_core::net::client::{Client, ClientConfig, ReconnectStrategy, UntypedClient};
use distant_core::net::common::{Destination, Map, SecretKey32, Version};
use distant_core::protocol::PROTOCOL_VERSION;
use log::*;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::watch;

#[inline]
fn missing(label: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("Missing {label}"))
}

#[inline]
fn invalid(label: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("Invalid {label}"))
}

/// Plugin for launching a local distant server process and connecting to distant TCP servers.
///
/// Handles the `"distant"` scheme. Launch spawns a local `distant server listen` process
/// and reads the resulting destination from its stdout. Connect establishes a TCP connection
/// to an already-running distant server, supporting both static key and challenge-based auth.
pub struct HostPlugin {
    shutdown: watch::Sender<bool>,
}

impl HostPlugin {
    /// Creates a new [`HostPlugin`].
    pub fn new() -> Self {
        Self {
            shutdown: watch::channel(false).0,
        }
    }

    /// Triggers shutdown of any tasks still checking that spawned servers have terminated.
    pub fn shutdown(&self) {
        let _ = self.shutdown.send(true);
    }

    async fn try_connect(
        ips: Vec<IpAddr>,
        port: u16,
        mut auth_handler: impl AuthHandler,
    ) -> io::Result<UntypedClient> {
        let mut err = None;
        for ip in ips {
            let addr = SocketAddr::new(ip, port);
            debug!("Attempting to connect to distant server @ {}", addr);

            match Client::tcp(addr)
                .auth_handler(DynAuthHandler::from(&mut auth_handler))
                .config(ClientConfig {
                    reconnect_strategy: ReconnectStrategy::ExponentialBackoff {
                        base: Duration::from_secs(1),
                        factor: 2.0,
                        max_duration: Some(Duration::from_secs(10)),
                        max_retries: None,
                        timeout: None,
                    },
                    ..Default::default()
                })
                .connect_timeout(Duration::from_secs(180))
                .version(Version::new(
                    PROTOCOL_VERSION.major,
                    PROTOCOL_VERSION.minor,
                    PROTOCOL_VERSION.patch,
                ))
                .connect_untyped()
                .await
            {
                Ok(client) => return Ok(client),
                Err(x) => err = Some(x),
            }
        }

        Err(err.expect("Err set above"))
    }
}

impl Default for HostPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for HostPlugin {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Plugin for HostPlugin {
    fn name(&self) -> &str {
        "distant"
    }

    fn connect<'a>(
        &'a self,
        raw_destination: &'a str,
        options: &'a Map,
        authenticator: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<UntypedClient>> + Send + 'a>> {
        Box::pin(async move {
            let destination = distant_core::parse_destination(raw_destination)?;
            debug!("Handling connect of {destination} with options '{options}'");
            let host = destination.host.to_string();
            let port = destination.port.ok_or_else(|| missing("port"))?;

            debug!("Looking up host {host} @ port {port}");
            let mut candidate_ips = tokio::net::lookup_host(format!("{host}:{port}"))
                .await
                .map_err(|x| {
                    io::Error::new(
                        x.kind(),
                        format!("{host} needs to be resolvable outside of ssh: {x}"),
                    )
                })?
                .map(|addr| addr.ip())
                .collect::<Vec<IpAddr>>();
            candidate_ips.sort_unstable();
            candidate_ips.dedup();
            if candidate_ips.is_empty() {
                return Err(io::Error::new(
                    io::ErrorKind::AddrNotAvailable,
                    format!("Unable to resolve {host}:{port}"),
                ));
            }

            // For legacy reasons, we need to support a static key being provided
            // via part of the destination OR an option, and attempt to use it
            // during authentication if it is provided
            if let Some(key) = destination
                .password
                .as_deref()
                .or_else(|| options.get("key").map(|s| s.as_str()))
            {
                let key = key.parse::<SecretKey32>().map_err(|_| invalid("key"))?;
                Self::try_connect(
                    candidate_ips,
                    port,
                    SingleAuthHandler::new(StaticKeyAuthMethodHandler::simple(key)),
                )
                .await
            } else {
                Self::try_connect(candidate_ips, port, ProxyAuthHandler::new(authenticator)).await
            }
        })
    }

    fn launch<'a>(
        &'a self,
        raw_destination: &'a str,
        options: &'a Map,
        _authenticator: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<Destination>> + Send + 'a>> {
        Box::pin(async move {
            let destination = distant_core::parse_destination(raw_destination)?;
            debug!("Handling launch of {destination} with options '{options}'");

            // Extract distant.* options directly from the Map
            let bin = options.get("distant.bin").cloned();
            let bind_server = options
                .get("distant.bind_server")
                .filter(|s| !s.eq_ignore_ascii_case("ssh"))
                .cloned()
                .unwrap_or_else(|| "any".to_string());
            let args = options.get("distant.args").cloned();

            // Get the path to the distant binary, ensuring it exists and is executable
            let program = which::which(match bin {
                Some(bin) => PathBuf::from(bin),
                None => std::env::current_exe().unwrap_or_else(|_| {
                    PathBuf::from(if cfg!(windows) {
                        "distant.exe"
                    } else {
                        "distant"
                    })
                }),
            })
            .map_err(|x| io::Error::new(io::ErrorKind::NotFound, x))?;

            // Build our command to run
            let mut cmd_args = vec![
                String::from("server"),
                String::from("listen"),
                String::from("--host"),
                bind_server,
            ];

            if let Some(port) = destination.port {
                cmd_args.push("--port".to_string());
                cmd_args.push(port.to_string());
            }

            // Add any options arguments to the command
            if let Some(options_args) = args {
                let mut distant_args = options_args.as_str();

                // Detect if our args are wrapped in quotes, and strip the outer quotes
                loop {
                    if let Some(a) = distant_args
                        .strip_prefix('"')
                        .and_then(|s| s.strip_suffix('"'))
                    {
                        distant_args = a;
                    } else if let Some(a) = distant_args
                        .strip_prefix('\'')
                        .and_then(|s| s.strip_suffix('\''))
                    {
                        distant_args = a;
                    } else {
                        break;
                    }
                }

                // NOTE: Split arguments based on whether we are running on windows or unix
                cmd_args.extend(if cfg!(windows) {
                    winsplit::split(distant_args)
                } else {
                    shell_words::split(distant_args)
                        .map_err(|x| io::Error::new(io::ErrorKind::InvalidInput, x))?
                });
            }

            // Spawn it and wait to get the communicated destination
            // NOTE: Server will persist until this plugin is dropped
            let mut command = Command::new(program);
            command
                .kill_on_drop(true)
                .args(cmd_args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            debug!("Launching local server by spawning command: {command:?}");
            let mut child = command.spawn()?;

            let mut stdout = BufReader::new(child.stdout.take().unwrap());

            let mut line = String::new();
            loop {
                match stdout.read_line(&mut line).await {
                    Ok(n) if n > 0 => {
                        if let Ok(destination) = line[..n].trim().parse::<Destination>() {
                            let mut rx = self.shutdown.subscribe();

                            // Wait for the process to complete in a task. We have to do this
                            // to properly check the exit status, otherwise if the server
                            // self-terminates then we get a ZOMBIE process! Oh no!
                            //
                            // This also replaces the need to store the children within the
                            // plugin itself and instead uses a watch update to kill the
                            // task in advance in the case where the child hasn't terminated.
                            tokio::spawn(async move {
                                // We don't actually care about the result, just that we're done
                                loop {
                                    tokio::select! {
                                        result = rx.changed() => {
                                            if result.is_err() {
                                                break;
                                            }

                                            if *rx.borrow_and_update() {
                                                break;
                                            }
                                        }
                                        _ = child.wait() => {
                                            break;
                                        }
                                    }
                                }
                            });

                            break Ok(destination);
                        } else {
                            line.clear();
                        }
                    }

                    // If we reach the point of no more data, then fail with EOF
                    Ok(_) => {
                        // Ensure that the server is terminated
                        child.kill().await?;

                        // Get any remaining output from the server's stderr to use for clues
                        let output = &child.wait_with_output().await?;
                        let stderr = String::from_utf8_lossy(&output.stderr);

                        break Err(io::Error::new(
                            io::ErrorKind::UnexpectedEof,
                            if stderr.trim().is_empty() {
                                "Missing output destination".to_string()
                            } else {
                                format!("Missing output destination due to error: {stderr}")
                            },
                        ));
                    }

                    // If we fail to read a line, we assume that the child has completed
                    // and we missed it, so capture the stderr to report issues
                    Err(x) => {
                        let output = child.wait_with_output().await?;
                        break Err(io::Error::other(
                            String::from_utf8(output.stderr).unwrap_or_else(|_| x.to_string()),
                        ));
                    }
                }
            }
        })
    }

    fn reconnect<'a>(
        &'a self,
        raw_destination: &'a str,
        options: &'a Map,
        authenticator: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<UntypedClient>> + Send + 'a>> {
        self.connect(raw_destination, options, authenticator)
    }

    fn reconnect_strategy(&self) -> ReconnectStrategy {
        ReconnectStrategy::ExponentialBackoff {
            base: Duration::from_secs(2),
            factor: 2.0,
            max_duration: Some(Duration::from_secs(30)),
            max_retries: Some(3),
            timeout: Some(Duration::from_secs(60)),
        }
    }
}

#[cfg(test)]
mod tests {
    //! Tests for helper functions (`missing`/`invalid`), HostPlugin construction,
    //! args quote-stripping, and `bind_server` SSH filtering via inline Map extraction.

    use test_log::test;

    use super::*;

    // -------------------------------------------------------
    // missing() / invalid() helpers
    // -------------------------------------------------------
    #[test]
    fn missing_creates_invalid_input_error() {
        let err = missing("port");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(err.to_string(), "Missing port");
    }

    #[test]
    fn invalid_creates_invalid_input_error() {
        let err = invalid("timeout");
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(err.to_string(), "Invalid timeout");
    }

    // -------------------------------------------------------
    // HostPlugin::new
    // -------------------------------------------------------
    #[test]
    fn host_plugin_new_does_not_panic() {
        let _plugin = HostPlugin::new();
    }

    // -------------------------------------------------------
    // HostPlugin::default
    // -------------------------------------------------------
    #[test]
    fn host_plugin_default_does_not_panic() {
        let _plugin = HostPlugin::default();
    }

    // -------------------------------------------------------
    // HostPlugin::name
    // -------------------------------------------------------
    #[test]
    fn host_plugin_name_is_distant() {
        let plugin = HostPlugin::new();
        assert_eq!(Plugin::name(&plugin), "distant");
    }

    // -------------------------------------------------------
    // HostPlugin::shutdown
    // -------------------------------------------------------
    #[test]
    fn host_plugin_shutdown_does_not_panic() {
        let plugin = HostPlugin::new();
        plugin.shutdown();
        // Call twice to ensure idempotent
        plugin.shutdown();
    }

    // -------------------------------------------------------
    // HostPlugin::drop calls shutdown
    // -------------------------------------------------------
    #[test]
    fn host_plugin_drop_does_not_panic() {
        let plugin = HostPlugin::new();
        drop(plugin);
    }

    // -------------------------------------------------------
    // launch — inline option extraction from Map
    // -------------------------------------------------------
    #[test]
    fn launch_options_extract_distant_fields() {
        let mut options = Map::new();
        options.insert("distant.bin".to_string(), "/usr/bin/distant".to_string());
        options.insert("distant.bind_server".to_string(), "127.0.0.1".to_string());
        options.insert("distant.args".to_string(), "--port 8080".to_string());

        let bin = options.get("distant.bin").cloned();
        let bind_server = options
            .get("distant.bind_server")
            .filter(|s| !s.eq_ignore_ascii_case("ssh"))
            .cloned()
            .unwrap_or_else(|| "any".to_string());
        let args = options.get("distant.args").cloned();

        assert_eq!(bin.as_deref(), Some("/usr/bin/distant"));
        assert_eq!(bind_server, "127.0.0.1");
        assert_eq!(args.as_deref(), Some("--port 8080"));
    }

    // -------------------------------------------------------
    // launch — args quote stripping logic
    // -------------------------------------------------------
    #[test]
    fn args_quote_stripping_double_quotes() {
        let mut distant_args: &str = "\"--port 8080\"";
        loop {
            if let Some(a) = distant_args
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
            {
                distant_args = a;
            } else if let Some(a) = distant_args
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
            {
                distant_args = a;
            } else {
                break;
            }
        }
        assert_eq!(distant_args, "--port 8080");
    }

    #[test]
    fn args_quote_stripping_single_quotes() {
        let mut distant_args: &str = "'--port 8080'";
        loop {
            if let Some(a) = distant_args
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
            {
                distant_args = a;
            } else if let Some(a) = distant_args
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
            {
                distant_args = a;
            } else {
                break;
            }
        }
        assert_eq!(distant_args, "--port 8080");
    }

    #[test]
    fn args_quote_stripping_nested_quotes() {
        let mut distant_args: &str = "\"'--port 8080'\"";
        loop {
            if let Some(a) = distant_args
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
            {
                distant_args = a;
            } else if let Some(a) = distant_args
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
            {
                distant_args = a;
            } else {
                break;
            }
        }
        assert_eq!(distant_args, "--port 8080");
    }

    #[test]
    fn args_quote_stripping_no_quotes() {
        let mut distant_args: &str = "--port 8080";
        loop {
            if let Some(a) = distant_args
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
            {
                distant_args = a;
            } else if let Some(a) = distant_args
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
            {
                distant_args = a;
            } else {
                break;
            }
        }
        assert_eq!(distant_args, "--port 8080");
    }

    // -------------------------------------------------------
    // bind_server SSH filtering via inline extraction
    // -------------------------------------------------------
    #[test]
    fn bind_server_ssh_is_filtered_to_any() {
        let mut options = Map::new();
        options.insert("distant.bind_server".to_string(), "ssh".to_string());

        let bind_server = options
            .get("distant.bind_server")
            .filter(|s| !s.eq_ignore_ascii_case("ssh"))
            .cloned()
            .unwrap_or_else(|| "any".to_string());
        assert_eq!(bind_server, "any");
    }

    #[test]
    fn bind_server_any_is_not_filtered() {
        let mut options = Map::new();
        options.insert("distant.bind_server".to_string(), "any".to_string());

        let bind_server = options
            .get("distant.bind_server")
            .filter(|s| !s.eq_ignore_ascii_case("ssh"))
            .cloned()
            .unwrap_or_else(|| "any".to_string());
        assert_eq!(bind_server, "any");
    }

    #[test]
    fn bind_server_host_is_not_filtered() {
        let mut options = Map::new();
        options.insert("distant.bind_server".to_string(), "example.com".to_string());

        let bind_server = options
            .get("distant.bind_server")
            .filter(|s| !s.eq_ignore_ascii_case("ssh"))
            .cloned()
            .unwrap_or_else(|| "any".to_string());
        assert_eq!(bind_server, "example.com");
    }

    #[test]
    fn bind_server_none_defaults_to_any() {
        let options = Map::new();

        let bind_server = options
            .get("distant.bind_server")
            .filter(|s| !s.eq_ignore_ascii_case("ssh"))
            .cloned()
            .unwrap_or_else(|| "any".to_string());
        assert_eq!(bind_server, "any");
    }

    #[test]
    fn host_plugin_reconnect_strategy_returns_exponential_backoff() {
        let plugin = HostPlugin::new();
        let strategy = Plugin::reconnect_strategy(&plugin);
        assert!(strategy.is_exponential_backoff());
        assert_eq!(strategy.max_retries(), Some(3));
        assert_eq!(strategy.max_duration(), Some(Duration::from_secs(30)));
        assert_eq!(strategy.timeout(), Some(Duration::from_secs(60)));
    }
}
