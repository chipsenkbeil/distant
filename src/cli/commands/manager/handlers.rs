use std::io;
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use async_trait::async_trait;
use distant_core::net::auth::msg::*;
use distant_core::net::auth::{
    AuthHandler, Authenticator, DynAuthHandler, ProxyAuthHandler, SingleAuthHandler,
    StaticKeyAuthMethodHandler,
};
use distant_core::net::client::{Client, ClientConfig, ReconnectStrategy, UntypedClient};
use distant_core::net::common::{Destination, Map, SecretKey32, Version};
use distant_core::net::manager::{ConnectHandler, LaunchHandler};
use distant_core::protocol::PROTOCOL_VERSION;
use log::*;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::{watch, Mutex};

use crate::options::{BindAddress, ClientLaunchConfig};

#[inline]
fn missing(label: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("Missing {label}"))
}

#[inline]
fn invalid(label: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("Invalid {label}"))
}

/// Supports launching locally through the manager as defined by `manager://...`
pub struct ManagerLaunchHandler {
    shutdown: watch::Sender<bool>,
}

impl ManagerLaunchHandler {
    pub fn new() -> Self {
        Self {
            shutdown: watch::channel(false).0,
        }
    }

    /// Triggers shutdown of any tasks still checking that spawned servers have terminated.
    pub fn shutdown(&self) {
        let _ = self.shutdown.send(true);
    }
}

impl Drop for ManagerLaunchHandler {
    /// Terminates waiting for any servers spawned by this handler, which in turn should
    /// shut them down.
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[async_trait]
impl LaunchHandler for ManagerLaunchHandler {
    async fn launch(
        &self,
        destination: &Destination,
        options: &Map,
        _authenticator: &mut dyn Authenticator,
    ) -> io::Result<Destination> {
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
        // NOTE: Server will persist until this handler is dropped
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
                        // handler itself and instead uses a watch update to kill the
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
                    break Err(io::Error::new(
                        io::ErrorKind::Other,
                        String::from_utf8(output.stderr).unwrap_or_else(|_| x.to_string()),
                    ));
                }
            }
        }
    }
}

/// Supports launching remotely via SSH as defined by `ssh://...`
#[cfg(any(feature = "libssh", feature = "ssh2"))]
pub struct SshLaunchHandler;

#[cfg(any(feature = "libssh", feature = "ssh2"))]
#[async_trait]
impl LaunchHandler for SshLaunchHandler {
    async fn launch(
        &self,
        destination: &Destination,
        options: &Map,
        authenticator: &mut dyn Authenticator,
    ) -> io::Result<Destination> {
        debug!("Handling launch of {destination} with options '{options}'");
        let config = ClientLaunchConfig::from(options.clone());

        use distant_ssh2::DistantLaunchOpts;
        let mut ssh = load_ssh(destination, options)?;
        let handler = AuthClientSshAuthHandler::new(authenticator);
        let _ = ssh.authenticate(handler).await?;
        let opts = {
            let opts = DistantLaunchOpts::default();
            DistantLaunchOpts {
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
    }
}

/// Supports connecting to a remote distant TCP server as defined by `distant://...`
pub struct DistantConnectHandler;

impl DistantConnectHandler {
    async fn try_connect(
        ips: Vec<IpAddr>,
        port: u16,
        mut auth_handler: impl AuthHandler,
    ) -> io::Result<UntypedClient> {
        // Try each IP address with the same port to see if one works
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

        // If all failed, return the last error we got
        Err(err.expect("Err set above"))
    }
}

#[async_trait]
impl ConnectHandler for DistantConnectHandler {
    async fn connect(
        &self,
        destination: &Destination,
        options: &Map,
        authenticator: &mut dyn Authenticator,
    ) -> io::Result<UntypedClient> {
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
    }
}

/// Supports connecting to a remote SSH server as defined by `ssh://...`
#[cfg(any(feature = "libssh", feature = "ssh2"))]
pub struct SshConnectHandler;

#[cfg(any(feature = "libssh", feature = "ssh2"))]
#[async_trait]
impl ConnectHandler for SshConnectHandler {
    async fn connect(
        &self,
        destination: &Destination,
        options: &Map,
        authenticator: &mut dyn Authenticator,
    ) -> io::Result<UntypedClient> {
        debug!("Handling connect of {destination} with options '{options}'");
        let mut ssh = load_ssh(destination, options)?;
        let handler = AuthClientSshAuthHandler::new(authenticator);
        let _ = ssh.authenticate(handler).await?;
        Ok(ssh.into_distant_client().await?.into_untyped_client())
    }
}

#[cfg(any(feature = "libssh", feature = "ssh2"))]
struct AuthClientSshAuthHandler<'a>(Mutex<&'a mut dyn Authenticator>);

#[cfg(any(feature = "libssh", feature = "ssh2"))]
impl<'a> AuthClientSshAuthHandler<'a> {
    pub fn new(authenticator: &'a mut dyn Authenticator) -> Self {
        Self(Mutex::new(authenticator))
    }
}

#[cfg(any(feature = "libssh", feature = "ssh2"))]
#[async_trait]
impl<'a> distant_ssh2::SshAuthHandler for AuthClientSshAuthHandler<'a> {
    async fn on_authenticate(&self, event: distant_ssh2::SshAuthEvent) -> io::Result<Vec<String>> {
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

    async fn on_verify_host(&self, host: &str) -> io::Result<bool> {
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

    async fn on_banner(&self, text: &str) {
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

    async fn on_error(&self, text: &str) {
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

#[cfg(any(feature = "libssh", feature = "ssh2"))]
fn load_ssh(destination: &Destination, options: &Map) -> io::Result<distant_ssh2::Ssh> {
    trace!("load_ssh({destination}, {options})");
    use distant_ssh2::{Ssh, SshOpts};

    let host = destination.host.to_string();

    let opts = SshOpts {
        backend: match options
            .get("backend")
            .or_else(|| options.get("ssh.backend"))
        {
            Some(s) => s.parse().map_err(|_| invalid("backend"))?,
            None => Default::default(),
        },

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
        {
            Some(s) => s.parse().map_err(|_| invalid("verbose"))?,
            None => false,
        },

        ..Default::default()
    };

    debug!("Connecting to {host} via ssh with {opts:?}");
    Ssh::connect(host, opts)
}
