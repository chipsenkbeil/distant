use async_trait::async_trait;
use distant_core::{
    net::{
        AuthClient, AuthQuestion, FramedTransport, IntoSplit, SecretKey32, TcpTransport,
        XChaCha20Poly1305Codec,
    },
    BoxedDistantReader, BoxedDistantWriter, BoxedDistantWriterReader, ConnectHandler, Destination,
    Extra, LaunchHandler,
};
use std::io;

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
        todo!()
    }
}
