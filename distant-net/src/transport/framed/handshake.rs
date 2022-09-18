use super::{
    BoxedCodec, ChainCodec, CompressionCodec, CompressionLevel, CompressionType, EncryptionCodec,
    EncryptionType, FramedTransport, HeapSecretKey, PlainCodec, Transport,
};
use crate::utils;
use log::*;
use serde::{Deserialize, Serialize};
use std::io;

mod on_choice;
mod on_handshake;

pub use on_choice::*;
pub use on_handshake::*;

/// Options from the server representing available methods to configure a framed transport
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HandshakeServerOptions {
    #[serde(rename = "c")]
    compression: Vec<CompressionType>,
    #[serde(rename = "e")]
    encryption: Vec<EncryptionType>,
}

/// Client choice representing the selected configuration for a framed transport
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HandshakeClientChoice {
    #[serde(rename = "c")]
    compression: Option<CompressionType>,
    #[serde(rename = "cl")]
    compression_level: Option<CompressionLevel>,
    #[serde(rename = "e")]
    encryption: Option<EncryptionType>,
}

/// Definition of the handshake to perform for a transport
#[derive(Debug)]
pub enum Handshake<T, const CAPACITY: usize> {
    /// Indicates that the handshake is being performed from the client-side
    Client {
        /// Secret key to use with encryption
        key: HeapSecretKey,

        /// Callback to invoke when receiving server options
        on_choice: OnHandshakeClientChoice,

        /// Callback to invoke when the handshake has completed, providing a user-level handshake
        /// operations
        on_handshake: OnHandshake<T, CAPACITY>,
    },

    /// Indicates that the handshake is being performed from the server-side
    Server {
        /// List of available compression algorithms for use between client and server
        compression: Vec<CompressionType>,

        /// List of available encryption algorithms for use between client and server
        encryption: Vec<EncryptionType>,

        /// Secret key to use with encryption
        key: HeapSecretKey,

        /// Callback to invoke when the handshake has completed, providing a user-level handshake
        /// operations
        on_handshake: OnHandshake<T, CAPACITY>,
    },
}

impl<T, const CAPACITY: usize> Handshake<T, CAPACITY> {
    /// Creates a new client handshake definition with `on_handshake` as a callback when the
    /// handshake has completed to enable user-level handshake operations
    pub fn client(
        key: HeapSecretKey,
        on_choice: impl Into<OnHandshakeClientChoice>,
        on_handshake: impl Into<OnHandshake<T, CAPACITY>>,
    ) -> Self {
        Self::Client {
            key,
            on_choice: on_choice.into(),
            on_handshake: on_handshake.into(),
        }
    }

    /// Creates a new server handshake definition with `on_handshake` as a callback when the
    /// handshake has completed to enable user-level handshake operations
    pub fn server(key: HeapSecretKey, on_handshake: impl Into<OnHandshake<T, CAPACITY>>) -> Self {
        Self::Server {
            compression: CompressionType::known_variants().to_vec(),
            encryption: EncryptionType::known_variants().to_vec(),
            key,
            on_handshake: on_handshake.into(),
        }
    }
}

/// Helper method to perform a handshake
///
/// ### Client
///
/// 1. Wait for options from server
/// 2. Send to server a compression and encryption choice
/// 3. Configure framed transport using selected choices
/// 4. Invoke on_handshake function
///
/// ### Server
///
/// 1. Send options to client
/// 2. Receive choices from client
/// 3. Configure framed transport using client's choices
/// 4. Invoke on_handshake function
///
pub(crate) async fn do_handshake<T, const CAPACITY: usize>(
    transport: T,
    handshake: &Handshake<T, CAPACITY>,
) -> io::Result<FramedTransport<T, CAPACITY>>
where
    T: Transport,
{
    let mut transport = FramedTransport::plain(transport);

    macro_rules! write_frame {
        ($data:expr) => {{
            transport
                .write_frame(utils::serialize_to_vec(&$data)?)
                .await?
        }};
    }

    macro_rules! next_frame_as {
        ($type:ty) => {{
            let frame = transport.read_frame().await?.ok_or_else(|| {
                io::Error::new(io::ErrorKind::UnexpectedEof, "Transport closed early")
            })?;

            utils::deserialize_from_slice::<$type>(frame.as_item())?
        }};
    }

    match handshake {
        Handshake::Client {
            key,
            on_choice,
            on_handshake,
        } => {
            // Receive options from the server and pick one
            debug!("[Handshake] Client waiting on server options");
            let options = next_frame_as!(HandshakeServerOptions);

            // Choose a compression and encryption option from the options
            debug!("[Handshake] Client selecting from server options: {options:#?}");
            let choice = (on_choice.0)(options);

            // Report back to the server the choice
            debug!("[Handshake] Client reporting choice: {choice:#?}");
            write_frame!(choice);

            // Transform the transport's codec to abide by the choice
            let transport = transform_transport(transport, choice, &key)?;

            // Invoke callback to signal completion of handshake
            debug!("[Handshake] Standard client handshake done, invoking callback");
            (on_handshake.0)(transport).await
        }
        Handshake::Server {
            compression,
            encryption,
            key,
            on_handshake,
        } => {
            let options = HandshakeServerOptions {
                compression: compression.to_vec(),
                encryption: encryption.to_vec(),
            };

            // Send options to the client
            debug!("[Handshake] Server sending options: {options:#?}");
            write_frame!(options);

            // Get client's response with selected compression and encryption
            debug!("[Handshake] Server waiting on client choice");
            let choice = next_frame_as!(HandshakeClientChoice);

            // Transform the transport's codec to abide by the choice
            let transport = transform_transport(transport, choice, &key)?;

            // Invoke callback to signal completion of handshake
            debug!("[Handshake] Standard server handshake done, invoking callback");
            (on_handshake.0)(transport).await
        }
    }
}

fn transform_transport<T, const CAPACITY: usize>(
    transport: FramedTransport<T, CAPACITY>,
    choice: HandshakeClientChoice,
    secret_key: &HeapSecretKey,
) -> io::Result<FramedTransport<T, CAPACITY>> {
    let codec: BoxedCodec = match (choice.compression, choice.encryption) {
        (Some(compression), Some(encryption)) => Box::new(ChainCodec::new(
            EncryptionCodec::from_type_and_key(encryption, secret_key.unprotected_as_bytes())?,
            CompressionCodec::from_type_and_level(
                compression,
                choice.compression_level.unwrap_or_default(),
            )?,
        )),
        (None, Some(encryption)) => Box::new(EncryptionCodec::from_type_and_key(
            encryption,
            secret_key.unprotected_as_bytes(),
        )?),
        (Some(compression), None) => Box::new(CompressionCodec::from_type_and_level(
            compression,
            choice.compression_level.unwrap_or_default(),
        )?),
        (None, None) => Box::new(PlainCodec::new()),
    };

    Ok(transport.with_codec(codec))
}
