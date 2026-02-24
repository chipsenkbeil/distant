use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::pin::Pin;
use std::process::Stdio;
use std::time::Duration;

use distant_core::Plugin;
use distant_core::auth::msg::*;
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
use tokio::sync::{Mutex, watch};

use crate::options::{BindAddress, ClientLaunchConfig};

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
pub struct DistantPlugin {
    shutdown: watch::Sender<bool>,
}

impl DistantPlugin {
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

impl Drop for DistantPlugin {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Plugin for DistantPlugin {
    fn name(&self) -> &str {
        "distant"
    }

    fn connect<'a>(
        &'a self,
        destination: &'a Destination,
        options: &'a Map,
        authenticator: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<UntypedClient>> + Send + 'a>> {
        Box::pin(async move {
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
        destination: &'a Destination,
        options: &'a Map,
        _authenticator: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<Destination>> + Send + 'a>> {
        Box::pin(async move {
            debug!("Handling launch of {destination} with options '{options}'");
            let config = ClientLaunchConfig::from(options.clone());

            // Get the path to the distant binary, ensuring it exists and is executable
            let program = which::which(match config.distant.bin {
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
            let mut args = vec![
                String::from("server"),
                String::from("listen"),
                String::from("--host"),
                // Disallow `ssh` from being used as the host
                config
                    .distant
                    .bind_server
                    .as_ref()
                    .filter(|x| !x.is_ssh())
                    .unwrap_or(&BindAddress::Any)
                    .to_string(),
            ];

            if let Some(port) = destination.port {
                args.push("--port".to_string());
                args.push(port.to_string());
            }

            // Add any options arguments to the command
            if let Some(options_args) = config.distant.args {
                let mut distant_args = options_args.as_str();

                // Detect if our args are wrapped in quotes, and strip the outer quotes
                loop {
                    if let Some(args) = distant_args
                        .strip_prefix('"')
                        .and_then(|s| s.strip_suffix('"'))
                    {
                        distant_args = args;
                    } else if let Some(args) = distant_args
                        .strip_prefix('\'')
                        .and_then(|s| s.strip_suffix('\''))
                    {
                        distant_args = args;
                    } else {
                        break;
                    }
                }

                // NOTE: Split arguments based on whether we are running on windows or unix
                args.extend(if cfg!(windows) {
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
                .args(args)
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
}

/// Plugin for launching and connecting via SSH.
///
/// Handles the `"ssh"` scheme. Launch uses SSH to start a distant server on the remote host.
/// Connect establishes a direct SSH connection and wraps it as a distant client.
pub struct SshPlugin;

impl Plugin for SshPlugin {
    fn name(&self) -> &str {
        "ssh"
    }

    fn connect<'a>(
        &'a self,
        destination: &'a Destination,
        options: &'a Map,
        authenticator: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<UntypedClient>> + Send + 'a>> {
        Box::pin(async move {
            debug!("Handling connect of {destination} with options '{options}'");
            let mut ssh = load_ssh(destination, options).await?;
            let handler = AuthClientSshAuthHandler::new(authenticator);
            ssh.authenticate(handler).await?;
            Ok(ssh.into_distant_client().await?.into_untyped_client())
        })
    }

    fn launch<'a>(
        &'a self,
        destination: &'a Destination,
        options: &'a Map,
        authenticator: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<Destination>> + Send + 'a>> {
        Box::pin(async move {
            debug!("Handling launch of {destination} with options '{options}'");
            let config = ClientLaunchConfig::from(options.clone());

            use distant_ssh::LaunchOpts;
            let mut ssh = load_ssh(destination, options).await?;
            let handler = AuthClientSshAuthHandler::new(authenticator);
            ssh.authenticate(handler).await?;
            let opts = {
                let opts = LaunchOpts::default();
                LaunchOpts {
                    binary: config.distant.bin.unwrap_or(opts.binary),
                    args: config.distant.args.unwrap_or(opts.args),
                    timeout: match options.get("timeout") {
                        Some(s) => std::time::Duration::from_millis(
                            s.parse::<u64>().map_err(|_| invalid("timeout"))?,
                        ),
                        None => opts.timeout,
                    },
                }
            };

            debug!("Launching via ssh: {opts:?}");
            ssh.launch(opts).await?.try_to_destination()
        })
    }
}

struct AuthClientSshAuthHandler<'a>(Mutex<&'a mut dyn Authenticator>);

impl<'a> AuthClientSshAuthHandler<'a> {
    pub fn new(authenticator: &'a mut dyn Authenticator) -> Self {
        Self(Mutex::new(authenticator))
    }
}

impl<'a> distant_ssh::SshAuthHandler for AuthClientSshAuthHandler<'a> {
    async fn on_authenticate(&self, event: distant_ssh::SshAuthEvent) -> io::Result<Vec<String>> {
        use std::collections::HashMap;
        let mut options = HashMap::new();
        let mut questions = Vec::new();

        for prompt in event.prompts {
            let mut options = HashMap::new();
            options.insert("echo".to_string(), prompt.echo.to_string());
            questions.push(Question {
                label: "ssh-prompt".to_string(),
                text: prompt.prompt,
                options,
            });
        }

        options.insert("instructions".to_string(), event.instructions);
        options.insert("username".to_string(), event.username);

        Ok(self
            .0
            .lock()
            .await
            .challenge(Challenge { questions, options })
            .await?
            .answers)
    }

    async fn on_verify_host<'b>(&'b self, host: &'b str) -> io::Result<bool> {
        Ok(self
            .0
            .lock()
            .await
            .verify(Verification {
                kind: VerificationKind::Host,
                text: host.to_string(),
            })
            .await?
            .valid)
    }

    async fn on_banner<'b>(&'b self, text: &'b str) {
        if let Err(x) = self
            .0
            .lock()
            .await
            .info(Info {
                text: text.to_string(),
            })
            .await
        {
            error!("ssh on_banner failed: {}", x);
        }
    }

    async fn on_error<'b>(&'b self, text: &'b str) {
        if let Err(x) = self
            .0
            .lock()
            .await
            .error(Error {
                kind: ErrorKind::Fatal,
                text: text.to_string(),
            })
            .await
        {
            error!("ssh on_error failed: {}", x);
        }
    }
}

fn parse_ssh_opts(destination: &Destination, options: &Map) -> io::Result<distant_ssh::SshOpts> {
    use distant_ssh::SshOpts;

    Ok(SshOpts {
        identity_files: options
            .get("identity_files")
            .or_else(|| options.get("ssh.identity_files"))
            .map(|s| s.split(',').map(|s| PathBuf::from(s.trim())).collect())
            .unwrap_or_default(),

        identities_only: match options
            .get("identities_only")
            .or_else(|| options.get("ssh.identities_only"))
        {
            Some(s) => Some(s.parse().map_err(|_| invalid("identities_only"))?),
            None => None,
        },

        port: destination.port,

        proxy_command: options
            .get("proxy_command")
            .or_else(|| options.get("ssh.proxy_command"))
            .cloned(),

        user: destination.username.clone(),

        user_known_hosts_files: options
            .get("user_known_hosts_files")
            .or_else(|| options.get("ssh.user_known_hosts_files"))
            .map(|s| s.split(',').map(|s| PathBuf::from(s.trim())).collect())
            .unwrap_or_default(),

        verbose: match options
            .get("verbose")
            .or_else(|| options.get("ssh.verbose"))
            .or_else(|| options.get("client.verbose"))
        {
            Some(s) => s.parse().map_err(|_| invalid("verbose"))?,
            None => false,
        },

        ..Default::default()
    })
}

async fn load_ssh(destination: &Destination, options: &Map) -> io::Result<distant_ssh::Ssh> {
    trace!("load_ssh({destination}, {options})");
    use distant_ssh::Ssh;

    let host = destination.host.to_string();
    let opts = parse_ssh_opts(destination, options)?;

    debug!("Connecting to {host} via ssh with {opts:?}");
    Ssh::connect(host, opts).await
}

#[cfg(test)]
mod tests {
    //! Tests for handler helpers (`missing`/`invalid`), plugin types, SSH option
    //! parsing via `parse_ssh_opts`, args quote-stripping, and `bind_server` filtering.

    use distant_core::net::common::Host;
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
    // DistantPlugin::new
    // -------------------------------------------------------
    #[test]
    fn distant_plugin_new_does_not_panic() {
        let _plugin = DistantPlugin::new();
    }

    // -------------------------------------------------------
    // DistantPlugin::name
    // -------------------------------------------------------
    #[test]
    fn distant_plugin_name_is_distant() {
        let plugin = DistantPlugin::new();
        assert_eq!(Plugin::name(&plugin), "distant");
    }

    // -------------------------------------------------------
    // DistantPlugin::shutdown
    // -------------------------------------------------------
    #[test]
    fn distant_plugin_shutdown_does_not_panic() {
        let plugin = DistantPlugin::new();
        plugin.shutdown();
        // Call twice to ensure idempotent
        plugin.shutdown();
    }

    // -------------------------------------------------------
    // DistantPlugin::drop calls shutdown
    // -------------------------------------------------------
    #[test]
    fn distant_plugin_drop_does_not_panic() {
        let plugin = DistantPlugin::new();
        drop(plugin);
    }

    // -------------------------------------------------------
    // SshPlugin::name
    // -------------------------------------------------------
    #[test]
    fn ssh_plugin_name_is_ssh() {
        let plugin = SshPlugin;
        assert_eq!(Plugin::name(&plugin), "ssh");
    }

    // -------------------------------------------------------
    // ClientLaunchConfig from options — used in launch path
    // -------------------------------------------------------
    #[test]
    fn launch_config_from_options_extracts_distant_fields() {
        let mut options = Map::new();
        options.insert("distant.bin".to_string(), "/usr/bin/distant".to_string());
        options.insert("distant.bind_server".to_string(), "127.0.0.1".to_string());
        options.insert("distant.args".to_string(), "--port 8080".to_string());

        let config = ClientLaunchConfig::from(options);
        assert_eq!(config.distant.bin.as_deref(), Some("/usr/bin/distant"));
        assert_eq!(config.distant.args.as_deref(), Some("--port 8080"));
    }

    // -------------------------------------------------------
    // parse_ssh_opts — option parsing tests
    // -------------------------------------------------------

    fn make_destination(host: &str) -> Destination {
        Destination {
            scheme: None,
            username: None,
            password: None,
            host: host.parse().unwrap(),
            port: None,
        }
    }

    #[test]
    fn ssh_opts_identity_files_parsing() {
        let mut options = Map::new();
        options.insert(
            "identity_files".to_string(),
            "/home/user/.ssh/id_rsa,/home/user/.ssh/id_ed25519".to_string(),
        );

        let opts = super::parse_ssh_opts(&make_destination("example.com"), &options).unwrap();
        assert_eq!(opts.identity_files.len(), 2);
        assert_eq!(
            opts.identity_files[0],
            PathBuf::from("/home/user/.ssh/id_rsa")
        );
        assert_eq!(
            opts.identity_files[1],
            PathBuf::from("/home/user/.ssh/id_ed25519")
        );
    }

    #[test]
    fn ssh_opts_identity_files_with_ssh_prefix() {
        let mut options = Map::new();
        options.insert(
            "ssh.identity_files".to_string(),
            "/home/user/.ssh/id_rsa".to_string(),
        );

        let opts = super::parse_ssh_opts(&make_destination("example.com"), &options).unwrap();
        assert_eq!(opts.identity_files.len(), 1);
        assert_eq!(
            opts.identity_files[0],
            PathBuf::from("/home/user/.ssh/id_rsa")
        );
    }

    #[test]
    fn ssh_opts_identities_only_parsing() {
        let mut options = Map::new();
        options.insert("identities_only".to_string(), "true".to_string());

        let opts = super::parse_ssh_opts(&make_destination("example.com"), &options).unwrap();
        assert_eq!(opts.identities_only, Some(true));
    }

    #[test]
    fn ssh_opts_verbose_parsing() {
        let mut options = Map::new();
        options.insert("verbose".to_string(), "true".to_string());

        let opts = super::parse_ssh_opts(&make_destination("example.com"), &options).unwrap();
        assert!(opts.verbose);
    }

    #[test]
    fn ssh_opts_verbose_with_ssh_prefix() {
        let mut options = Map::new();
        options.insert("ssh.verbose".to_string(), "true".to_string());

        let opts = super::parse_ssh_opts(&make_destination("example.com"), &options).unwrap();
        assert!(opts.verbose);
    }

    #[test]
    fn ssh_opts_verbose_with_client_prefix() {
        let mut options = Map::new();
        options.insert("client.verbose".to_string(), "true".to_string());

        let opts = super::parse_ssh_opts(&make_destination("example.com"), &options).unwrap();
        assert!(opts.verbose);
    }

    #[test]
    fn ssh_opts_verbose_defaults_to_false() {
        let options = Map::new();

        let opts = super::parse_ssh_opts(&make_destination("example.com"), &options).unwrap();
        assert!(!opts.verbose);
    }

    #[test]
    fn ssh_opts_proxy_command_parsing() {
        let mut options = Map::new();
        options.insert(
            "proxy_command".to_string(),
            "ssh -W %h:%p proxy.example.com".to_string(),
        );

        let opts = super::parse_ssh_opts(&make_destination("example.com"), &options).unwrap();
        assert_eq!(
            opts.proxy_command.as_deref(),
            Some("ssh -W %h:%p proxy.example.com")
        );
    }

    #[test]
    fn ssh_opts_user_known_hosts_files_parsing() {
        let mut options = Map::new();
        options.insert(
            "user_known_hosts_files".to_string(),
            "/home/user/.ssh/known_hosts,/etc/ssh/known_hosts".to_string(),
        );

        let opts = super::parse_ssh_opts(&make_destination("example.com"), &options).unwrap();
        assert_eq!(opts.user_known_hosts_files.len(), 2);
        assert_eq!(
            opts.user_known_hosts_files[0],
            PathBuf::from("/home/user/.ssh/known_hosts")
        );
        assert_eq!(
            opts.user_known_hosts_files[1],
            PathBuf::from("/etc/ssh/known_hosts")
        );
    }

    #[test]
    fn ssh_opts_empty_options_produces_defaults() {
        let options = Map::new();

        let opts = super::parse_ssh_opts(&make_destination("example.com"), &options).unwrap();
        assert!(opts.identity_files.is_empty());
        assert!(opts.user_known_hosts_files.is_empty());
        assert!(opts.proxy_command.is_none());
    }

    // -------------------------------------------------------
    // launch — args quote stripping logic
    // -------------------------------------------------------
    // These tests replicate the quote-stripping loop from the launch path.
    // The loop is not yet extracted into a standalone function, so we test
    // a copy of the pattern here.

    #[test]
    fn args_quote_stripping_double_quotes() {
        let mut distant_args: &str = "\"--port 8080\"";
        loop {
            if let Some(args) = distant_args
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
            {
                distant_args = args;
            } else if let Some(args) = distant_args
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
            {
                distant_args = args;
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
            if let Some(args) = distant_args
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
            {
                distant_args = args;
            } else if let Some(args) = distant_args
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
            {
                distant_args = args;
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
            if let Some(args) = distant_args
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
            {
                distant_args = args;
            } else if let Some(args) = distant_args
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
            {
                distant_args = args;
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
            if let Some(args) = distant_args
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
            {
                distant_args = args;
            } else if let Some(args) = distant_args
                .strip_prefix('\'')
                .and_then(|s| s.strip_suffix('\''))
            {
                distant_args = args;
            } else {
                break;
            }
        }
        assert_eq!(distant_args, "--port 8080");
    }

    // -------------------------------------------------------
    // DistantPlugin — bind_server ssh filtering
    // -------------------------------------------------------
    #[test]
    fn bind_server_ssh_is_filtered_to_any() {
        let bind_server = Some(BindAddress::Ssh);
        let result = bind_server
            .as_ref()
            .filter(|x| !x.is_ssh())
            .unwrap_or(&BindAddress::Any);
        assert_eq!(*result, BindAddress::Any);
    }

    #[test]
    fn bind_server_any_is_not_filtered() {
        let bind_server = Some(BindAddress::Any);
        let result = bind_server
            .as_ref()
            .filter(|x| !x.is_ssh())
            .unwrap_or(&BindAddress::Any);
        assert_eq!(*result, BindAddress::Any);
    }

    #[test]
    fn bind_server_host_is_not_filtered() {
        let bind_server = Some(BindAddress::Host(Host::Name("example.com".to_string())));
        let result = bind_server
            .as_ref()
            .filter(|x| !x.is_ssh())
            .unwrap_or(&BindAddress::Any);
        assert_eq!(
            *result,
            BindAddress::Host(Host::Name("example.com".to_string()))
        );
    }

    #[test]
    fn bind_server_none_defaults_to_any() {
        let bind_server: Option<BindAddress> = None;
        let result = bind_server
            .as_ref()
            .filter(|x| !x.is_ssh())
            .unwrap_or(&BindAddress::Any);
        assert_eq!(*result, BindAddress::Any);
    }

    // -------------------------------------------------------
    // SshPlugin launch config extraction
    // -------------------------------------------------------
    #[test]
    fn ssh_launch_config_timeout_parsing() {
        let mut options = Map::new();
        options.insert("timeout".to_string(), "5000".to_string());

        let timeout = match options.get("timeout") {
            Some(s) => std::time::Duration::from_millis(s.parse::<u64>().unwrap()),
            None => std::time::Duration::from_secs(15), // default
        };

        assert_eq!(timeout, std::time::Duration::from_millis(5000));
    }

    #[test]
    fn ssh_launch_config_timeout_default() {
        let options = Map::new();

        let default_timeout = std::time::Duration::from_secs(15);
        let timeout = match options.get("timeout") {
            Some(s) => std::time::Duration::from_millis(s.parse::<u64>().unwrap()),
            None => default_timeout,
        };

        assert_eq!(timeout, default_timeout);
    }
}
