use crate::{BoxedCodec, FramedTransport, PlainCodec, Request, Response, Transport};
use serde::{Deserialize, Serialize};
use std::io;

/// Represents options that the server has available for a connection
#[derive(Serialize, Deserialize)]
struct ServerConnectionOptions {
    /// Choices for encryption as string labels
    pub encryption: Vec<String>,

    /// Choices for compression as string labels
    pub compression: Vec<String>,
}

/// Represents the choice that the client has made regarding server connection options
struct ClientConnectionChoice {
    /// Selected encryption
    pub encryption: String,

    /// Selected compression
    pub compression: String,
}

/// Performs the client-side of a handshake
pub async fn client_handshake<T>(transport: T) -> io::Result<FramedTransport<T, BoxedCodec>>
where
    T: Transport,
{
    let transport = FramedTransport::new(transport, PlainCodec::new());

    // Wait for the server to send us choices for communication
    let frame = transport.read_frame().await?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "Connection aborted before receiving server communication",
        )
    })?;

    // Parse the frame as the request for the client
    let request = Request::<ServerConnectionOptions>::from_slice(frame.as_item())?;

    // Select an encryption and compression choice
    let encryption = request.payload.encryption[0];
    let compression = request.payload.compression[0];

    // Respond back with choices
}

/// Performs the server-side of a handshake
pub async fn server_handshake<T>(transport: T) -> io::Result<FramedTransport<T, BoxedCodec>>
where
    T: Transport,
{
    let transport = FramedTransport::new(transport, PlainCodec::new());
}
