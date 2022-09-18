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
    compression_types: Vec<CompressionType>,
    #[serde(rename = "e")]
    encryption_types: Vec<EncryptionType>,
}

/// Client choice representing the selected configuration for a framed transport
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HandshakeClientChoice {
    #[serde(rename = "c")]
    compression_type: Option<CompressionType>,
    #[serde(rename = "cl")]
    compression_level: Option<CompressionLevel>,
    #[serde(rename = "e")]
    encryption_type: Option<EncryptionType>,
}

/// Definition of the handshake to perform for a transport
#[derive(Clone, Debug)]
pub enum Handshake {
    /// Indicates that the handshake is being performed from the client-side
    Client {
        /// Secret key to use with encryption
        key: HeapSecretKey,

        /// Preferred compression algorithm when presented options by server
        preferred_compression_type: Option<CompressionType>,

        /// Preferred compression level when presented options by server
        preferred_compression_level: Option<CompressionLevel>,

        /// Preferred encryption algorithm when presented options by server
        preferred_encryption_type: Option<EncryptionType>,
    },

    /// Indicates that the handshake is being performed from the server-side
    Server {
        /// Secret key to use with encryption
        key: HeapSecretKey,

        /// List of available compression algorithms for use between client and server
        compression_types: Vec<CompressionType>,

        /// List of available encryption algorithms for use between client and server
        encryption_types: Vec<EncryptionType>,
    },
}

impl Handshake {
    /// Creates a new client handshake definition, using `key` for encryption, providing defaults
    /// for the preferred compression type, compression level, and encryption type
    pub fn client(key: HeapSecretKey) -> Self {
        Self::Client {
            key,
            preferred_compression_type: None,
            preferred_compression_level: None,
            preferred_encryption_type: Some(EncryptionType::XChaCha20Poly1305),
        }
    }

    /// Creates a new client handshake definition, using `key` for encryption, providing defaults
    /// for the compression types and encryption types by including all known variants
    pub fn server(key: HeapSecretKey) -> Self {
        Self::Server {
            compression_types: CompressionType::known_variants().to_vec(),
            encryption_types: EncryptionType::known_variants().to_vec(),
            key,
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
    transport: &mut FramedTransport<T, CAPACITY>,
) -> io::Result<()>
where
    T: Transport,
{
    // Place transport in plain text communication mode for start of handshake, and clear any data
    // that is lingering within internal buffers
    transport.set_codec(Box::new(PlainCodec::new()));
    transport.clear();

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

    match transport.handshake.clone() {
        Handshake::Client {
            key,
            preferred_compression_type,
            preferred_compression_level,
            preferred_encryption_type,
        } => {
            // Receive options from the server and pick one
            debug!("[Handshake] Client waiting on server options");
            let options = next_frame_as!(HandshakeServerOptions);

            // Choose a compression and encryption option from the options
            debug!("[Handshake] Client selecting from server options: {options:#?}");
            let choice = HandshakeClientChoice {
                compression_type: preferred_compression_type
                    .filter(|ty| options.compression_types.contains(ty)),
                compression_level: preferred_compression_level,
                encryption_type: preferred_encryption_type
                    .filter(|ty| options.encryption_types.contains(ty)),
            };

            // Report back to the server the choice
            debug!("[Handshake] Client reporting choice: {choice:#?}");
            write_frame!(choice);

            // Transform the transport's codec to abide by the choice
            debug!("[Handshake] Client updating codec based on {choice:#?}");
            transform_transport(transport, choice, &key)
        }
        Handshake::Server {
            key,
            compression_types,
            encryption_types,
        } => {
            let options = HandshakeServerOptions {
                compression_types: compression_types.to_vec(),
                encryption_types: encryption_types.to_vec(),
            };

            // Send options to the client
            debug!("[Handshake] Server sending options: {options:#?}");
            write_frame!(options);

            // Get client's response with selected compression and encryption
            debug!("[Handshake] Server waiting on client choice");
            let choice = next_frame_as!(HandshakeClientChoice);

            // Transform the transport's codec to abide by the choice
            debug!("[Handshake] Server updating codec based on {choice:#?}");
            transform_transport(transport, choice, &key)
        }
    }
}

fn transform_transport<T, const CAPACITY: usize>(
    transport: &mut FramedTransport<T, CAPACITY>,
    choice: HandshakeClientChoice,
    secret_key: &HeapSecretKey,
) -> io::Result<()> {
    let codec: BoxedCodec = match (choice.compression_type, choice.encryption_type) {
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

    Ok(transport.set_codec(codec))
}
