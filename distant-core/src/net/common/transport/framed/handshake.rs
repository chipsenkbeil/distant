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

    /// Returns true if handshake is from client-side
    pub fn is_client(&self) -> bool {
        matches!(self, Self::Client { .. })
    }

    /// Returns true if handshake is from server-side
    pub fn is_server(&self) -> bool {
        matches!(self, Self::Server { .. })
    }
}

#[cfg(test)]
mod tests {
    //! Tests for Handshake: client/server variant discrimination and default field values
    //! for compression and encryption preferences.

    use super::*;

    #[test]
    fn client_handshake_is_client() {
        let hs = Handshake::client();
        assert!(hs.is_client());
        assert!(!hs.is_server());
    }

    #[test]
    fn server_handshake_is_server() {
        let hs = Handshake::server();
        assert!(hs.is_server());
        assert!(!hs.is_client());
    }

    #[test]
    fn client_handshake_defaults() {
        match Handshake::client() {
            Handshake::Client {
                preferred_compression_type,
                preferred_compression_level,
                preferred_encryption_type,
            } => {
                assert!(preferred_compression_type.is_none());
                assert!(preferred_compression_level.is_none());
                assert_eq!(
                    preferred_encryption_type,
                    Some(EncryptionType::XChaCha20Poly1305)
                );
            }
            _ => panic!("Expected Client variant"),
        }
    }

    #[test]
    fn server_handshake_defaults() {
        match Handshake::server() {
            Handshake::Server {
                compression_types,
                encryption_types,
            } => {
                assert_eq!(compression_types, CompressionType::known_variants());
                assert_eq!(encryption_types, EncryptionType::known_variants());
            }
            _ => panic!("Expected Server variant"),
        }
    }
}
