use super::{CompressionLevel, CompressionType, EncryptionType};

/// Definition of the handshake to perform for a transport
#[derive(Clone, Debug)]
pub enum Handshake {
    /// Indicates that the handshake is being performed from the client-side
    Client {
        /// Preferred compression algorithm when presented options by server
        preferred_compression_type: Option<CompressionType>,

        /// Preferred compression level when presented options by server
        preferred_compression_level: Option<CompressionLevel>,

        /// Preferred encryption algorithm when presented options by server
        preferred_encryption_type: Option<EncryptionType>,
    },

    /// Indicates that the handshake is being performed from the server-side
    Server {
        /// List of available compression algorithms for use between client and server
        compression_types: Vec<CompressionType>,

        /// List of available encryption algorithms for use between client and server
        encryption_types: Vec<EncryptionType>,
    },
}

impl Handshake {
    /// Creates a new client handshake definition, providing defaults for the preferred compression
    /// type, compression level, and encryption type
    pub fn client() -> Self {
        Self::Client {
            preferred_compression_type: None,
            preferred_compression_level: None,
            preferred_encryption_type: Some(EncryptionType::XChaCha20Poly1305),
        }
    }

    /// Creates a new server handshake definition, providing defaults for the compression types and
    /// encryption types by including all known variants
    pub fn server() -> Self {
        Self::Server {
            compression_types: CompressionType::known_variants().to_vec(),
            encryption_types: EncryptionType::known_variants().to_vec(),
        }
    }
}
