use crate::config::ClientLaunchConfig;
use async_trait::async_trait;
use distant_core::{
    net::{
        AuthClient, AuthErrorKind, AuthQuestion, AuthVerifyKind, FramedTransport, IntoSplit,
        SecretKey32, TcpTransport, XChaCha20Poly1305Codec,
    },
    BoxedDistantReader, BoxedDistantWriter, BoxedDistantWriterReader, ConnectHandler, Destination,
    Extra, LaunchHandler,
};
use log::*;
use std::{
    collections::HashMap,
    io,
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::Mutex,
};

#[inline]
fn missing(label: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("Missing {}", label))
}

#[inline]
fn invalid(label: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("Invalid {}", label))
}

/// Supports launching locally through the manager as defined by `manager://...`
pub struct ManagerLaunchHandler;

#[async_trait]
impl LaunchHandler for ManagerLaunchHandler {
    async fn launch(
        &self,
        destination: &Destination,
        extra: &Extra,
        _auth_client: &mut AuthClient,
    ) -> io::Result<Destination> {
        let config = ClientLaunchConfig::from(extra.clone());

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
            config
                .distant
                .bind_server
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| String::from("any")),
        ];

        if let Some(port) = destination.port() {
            args.push("--port".to_string());
            args.push(port.to_string());
        }

        // Add any extra arguments to the command
        if let Some(extra_args) = config.distant.args {
            args.extend(
                shell_words::split(&extra_args)
                    .map_err(|x| io::Error::new(io::ErrorKind::InvalidInput, x))?,
            );
        }

        // Spawn it and wait to get the communicated destination
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let mut stdout = BufReader::new(child.stdout.take().unwrap());

        let mut line = String::new();
        loop {
            match stdout.read_line(&mut line).await {
                Ok(n) if n > 0 => {
                    if let Ok(destination) = line[..n].trim().parse::<Destination>() {
                        break Ok(destination);
                    } else {
                        line.clear();
                    }
                }

                // If we reach the point of no more data, then fail with EOF
                Ok(_) => {
                    break Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "Missing output destination",
                    ))
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
pub struct SshLaunchHandler;

#[async_trait]
impl LaunchHandler for SshLaunchHandler {
    async fn launch(
        &self,
        destination: &Destination,
        extra: &Extra,
        auth_client: &mut AuthClient,
    ) -> io::Result<Destination> {
        let config = ClientLaunchConfig::from(extra.clone());

        use distant_ssh2::DistantLaunchOpts;
        let mut ssh = load_ssh(destination, extra)?;
        let handler = AuthClientSshAuthHandler::new(auth_client);
        let _ = ssh.authenticate(handler).await?;
        let opts = {
            let opts = DistantLaunchOpts::default();
            DistantLaunchOpts {
                binary: config.distant.bin.unwrap_or(opts.binary),
                args: config.distant.args.unwrap_or(opts.args),
                use_login_shell: !config.distant.no_shell,
                timeout: match extra.get("timeout") {
                    Some(s) => {
                        Duration::from_millis(s.parse::<u64>().map_err(|_| invalid("timeout"))?)
                    }
                    None => opts.timeout,
                },
            }
        };
        ssh.launch(opts).await?.try_to_destination()
    }
}

/// Supports connecting to a remote distant TCP server as defined by `distant://...`
pub struct DistantConnectHandler;

impl DistantConnectHandler {
    pub async fn try_connect(ips: Vec<IpAddr>, port: u16) -> io::Result<TcpTransport> {
        // Try each IP address with the same port to see if one works
        let mut err = None;
        for ip in ips {
            let addr = SocketAddr::new(ip, port);
            debug!("Attempting to connect to distant server @ {}", addr);
            match TcpTransport::connect(addr).await {
                Ok(transport) => return Ok(transport),
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
        extra: &Extra,
        auth_client: &mut AuthClient,
    ) -> io::Result<BoxedDistantWriterReader> {
        let host = destination.to_host_string();
        let port = destination.port().ok_or_else(|| missing("port"))?;
        let mut candidate_ips = tokio::net::lookup_host(format!("{}:{}", host, port))
            .await
            .map_err(|x| {
                io::Error::new(
                    x.kind(),
                    format!("{} needs to be resolvable outside of ssh: {}", host, x),
                )
            })?
            .into_iter()
            .map(|addr| addr.ip())
            .collect::<Vec<IpAddr>>();
        candidate_ips.sort_unstable();
        candidate_ips.dedup();
        if candidate_ips.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                format!("Unable to resolve {}:{}", host, port),
            ));
        }

        // Use provided password or extra key if available, otherwise ask for it, and produce a
        // codec using the key
        let codec = {
            let key = destination
                .password()
                .or_else(|| extra.get("key").map(|s| s.as_str()));

            let key = match key {
                Some(key) => key.parse::<SecretKey32>().map_err(|_| invalid("key"))?,
                None => {
                    let answers = auth_client
                        .challenge(vec![AuthQuestion::new("key")], Default::default())
                        .await?;
                    answers
                        .first()
                        .ok_or_else(|| missing("key"))?
                        .parse::<SecretKey32>()
                        .map_err(|_| invalid("key"))?
                }
            };
            XChaCha20Poly1305Codec::from(key)
        };

        // Establish a TCP connection, wrap it, and split it out into a writer and reader
        let transport = Self::try_connect(candidate_ips, port).await?;
        let transport = FramedTransport::new(transport, codec);
        let (writer, reader) = transport.into_split();
        let writer: BoxedDistantWriter = Box::new(writer);
        let reader: BoxedDistantReader = Box::new(reader);
        Ok((writer, reader))
    }
}

/// Supports connecting to a remote SSH server as defined by `ssh://...`
pub struct SshConnectHandler;

#[async_trait]
impl ConnectHandler for SshConnectHandler {
    async fn connect(
        &self,
        destination: &Destination,
        extra: &Extra,
        auth_client: &mut AuthClient,
    ) -> io::Result<BoxedDistantWriterReader> {
        let mut ssh = load_ssh(destination, extra)?;
        let handler = AuthClientSshAuthHandler::new(auth_client);
        let _ = ssh.authenticate(handler).await?;
        ssh.into_distant_writer_reader().await
    }
}

struct AuthClientSshAuthHandler<'a>(Mutex<&'a mut AuthClient>);

impl<'a> AuthClientSshAuthHandler<'a> {
    pub fn new(auth_client: &'a mut AuthClient) -> Self {
        Self(Mutex::new(auth_client))
    }
}

#[async_trait]
impl<'a> distant_ssh2::SshAuthHandler for AuthClientSshAuthHandler<'a> {
    async fn on_authenticate(&self, event: distant_ssh2::SshAuthEvent) -> io::Result<Vec<String>> {
        let mut extra = HashMap::new();
        let mut questions = Vec::new();

        for prompt in event.prompts {
            let mut extra = HashMap::new();
            extra.insert("echo".to_string(), prompt.echo.to_string());
            questions.push(AuthQuestion {
                text: prompt.prompt,
                extra,
            });
        }

        extra.insert("instructions".to_string(), event.instructions);
        extra.insert("username".to_string(), event.username);

        self.0.lock().await.challenge(questions, extra).await
    }

    async fn on_verify_host(&self, host: &str) -> io::Result<bool> {
        self.0
            .lock()
            .await
            .verify(AuthVerifyKind::Host, host.to_string())
            .await
    }

    async fn on_banner(&self, text: &str) {
        if let Err(x) = self.0.lock().await.info(text.to_string()).await {
            error!("ssh on_banner failed: {}", x);
        }
    }

    async fn on_error(&self, text: &str) {
        if let Err(x) = self
            .0
            .lock()
            .await
            .error(AuthErrorKind::Unknown, text.to_string())
            .await
        {
            error!("ssh on_error failed: {}", x);
        }
    }
}

fn load_ssh(destination: &Destination, extra: &Extra) -> io::Result<distant_ssh2::Ssh> {
    use distant_ssh2::{Ssh, SshOpts};

    let host = destination.to_host_string();

    let opts = SshOpts {
        backend: match extra.get("backend") {
            Some(s) => s.parse().map_err(|_| invalid("backend"))?,
            None => Default::default(),
        },

        identity_files: extra
            .get("identity_files")
            .map(|s| s.split(',').map(|s| PathBuf::from(s.trim())).collect())
            .unwrap_or_default(),

        identities_only: match extra.get("identities_only") {
            Some(s) => Some(s.parse().map_err(|_| invalid("identities_only"))?),
            None => None,
        },

        port: destination.port(),

        proxy_command: extra.get("proxy_command").cloned(),

        user: destination.username().map(ToString::to_string),

        user_known_hosts_files: extra
            .get("user_known_hosts_files")
            .map(|s| s.split(',').map(|s| PathBuf::from(s.trim())).collect())
            .unwrap_or_default(),

        verbose: match extra.get("verbose") {
            Some(s) => s.parse().map_err(|_| invalid("verbose"))?,
            None => false,
        },

        ..Default::default()
    };
    Ssh::connect(host, opts)
}
