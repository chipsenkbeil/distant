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
use std::{collections::HashMap, io, path::PathBuf, time::Duration};
use tokio::sync::Mutex;

#[inline]
fn missing(label: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("Missing {}", label))
}

#[inline]
fn invalid(label: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, format!("Invalid {}", label))
}

/// Supports launching locally as defined by `local://...`
pub struct LocalLaunchHandler;

#[async_trait]
impl LaunchHandler for LocalLaunchHandler {
    async fn launch(
        &self,
        destination: &Destination,
        extra: &Extra,
        auth_client: &mut AuthClient,
    ) -> io::Result<Destination> {
        todo!()
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
        use distant_ssh2::DistantLaunchOpts;
        let mut ssh = load_ssh(destination, extra)?;
        let handler = AuthClientSshAuthHandler::new(auth_client);
        let _ = ssh.authenticate(handler).await?;
        let opts = {
            let opts = DistantLaunchOpts::default();
            DistantLaunchOpts {
                binary: extra
                    .get("binary")
                    .map(ToString::to_string)
                    .unwrap_or(opts.binary),
                args: extra
                    .get("args")
                    .map(ToString::to_string)
                    .unwrap_or(opts.args),
                use_login_shell: match extra.get("use_login_shell") {
                    Some(s) => s.parse().map_err(|_| invalid("use_login_shell"))?,
                    None => opts.use_login_shell,
                },
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

#[async_trait]
impl ConnectHandler for DistantConnectHandler {
    async fn connect(
        &self,
        destination: &Destination,
        extra: &Extra,
        auth_client: &mut AuthClient,
    ) -> io::Result<BoxedDistantWriterReader> {
        // Build address like `example.com:8080`
        let addr = format!(
            "{}:{}",
            destination.host().ok_or_else(|| missing("host"))?,
            destination.port().ok_or_else(|| missing("port"))?
        );

        // Use provided password or extra key if available, otherwise ask for it, and produce a
        // codec using the key
        let codec = {
            let key = destination
                .password()
                .map(|p| p.as_str())
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
        let transport = TcpTransport::connect(addr).await?;
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

    let host = destination
        .host()
        .map(ToString::to_string)
        .ok_or_else(|| missing("host"))?;

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
