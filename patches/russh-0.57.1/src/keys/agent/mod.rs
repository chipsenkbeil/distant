use std::borrow::Cow;

use ssh_key::{Certificate, PublicKey};

/// Write clients for SSH agents.
pub mod client;
mod msg;
/// Write servers for SSH agents.
pub mod server;

/// Constraints on how keys can be used
#[derive(Debug, PartialEq, Eq)]
pub enum Constraint {
    /// The key shall disappear from the agent's memory after that many seconds.
    KeyLifetime { seconds: u32 },
    /// Signatures need to be confirmed by the agent (for instance using a dialog).
    Confirm,
    /// Custom constraints
    Extensions { name: Vec<u8>, details: Vec<u8> },
}

/// An identity held by an SSH agent, which may be either a plain public key
/// or an OpenSSH certificate.
#[derive(Debug, Clone)]
#[allow(clippy::large_enum_variant)]
pub enum AgentIdentity {
    /// A plain public key
    PublicKey {
        /// The public key
        key: PublicKey,
        /// Comment associated with this identity
        comment: String,
    },
    /// An OpenSSH certificate
    Certificate {
        /// The certificate (contains public key plus CA signature, principals, validity, etc.)
        cert: Certificate,
        /// Comment associated with this identity
        comment: String,
    },
}

impl AgentIdentity {
    /// Returns the underlying public key.
    /// For certificates, extracts the public key from the certificate.
    pub fn public_key(&self) -> Cow<'_, PublicKey> {
        match self {
            Self::PublicKey { key, .. } => Cow::Borrowed(key),
            Self::Certificate { cert, .. } => {
                Cow::Owned(PublicKey::new(cert.public_key().clone(), ""))
            }
        }
    }

    /// Returns the comment associated with this identity.
    pub fn comment(&self) -> &str {
        match self {
            Self::PublicKey { comment, .. } => comment,
            Self::Certificate { comment, .. } => comment,
        }
    }
}
