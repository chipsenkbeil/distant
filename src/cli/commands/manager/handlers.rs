use async_trait::async_trait;
use distant_core::{
    net::{
        AuthClient, AuthQuestion, FramedTransport, IntoSplit, SecretKey32, TcpTransport,
        XChaCha20Poly1305Codec,
    },
    BoxedDistantReader, BoxedDistantWriter, BoxedDistantWriterReader, ConnectHandler, Destination,
    Extra, LaunchHandler,
};
use std::{io, path::PathBuf};

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
        use distant_ssh2::SshAuthHandler;

        let ssh = load_ssh(destination, extra)?;
        todo!()
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
        use distant_ssh2::SshAuthHandler;

        let mut ssh = load_ssh(destination, extra)?;

        // TODO: Need to support async functions
        let handler = SshAuthHandler {
            on_authenticate: Box::new(|ev| async {}),
            on_banner: Box::new(|text| async {}),
            on_host_verify: Box::new(|host| async {}),
            on_error: Box::new(|text| async {}),
        };

        let _ = ssh.authenticate(handler).await?;

        // TODO: Need to create another method that just splits and does not produce a client
        ssh.into_distant_writer_reader().await
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
