use derive_more::Display;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

mod client;
pub use client::*;

mod handshake;
pub use handshake::*;

mod server;
pub use server::*;

/// Represents authentication messages that can be sent over the wire
///
/// NOTE: Must use serde's content attribute with the tag attribute. Just the tag attribute will
///       cause deserialization to fail
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
pub enum Auth {
    /// Represents a request to perform an authentication handshake,
    /// providing the public key and salt from one side in order to
    /// derive the shared key
    #[serde(rename = "auth_handshake")]
    Handshake {
        /// Bytes of the public key
        #[serde(with = "serde_bytes")]
        public_key: PublicKeyBytes,

        /// Randomly generated salt
        #[serde(with = "serde_bytes")]
        salt: Salt,
    },

    /// Represents the bytes of an encrypted message
    ///
    /// Underneath, will be one of either [`AuthRequest`] or [`AuthResponse`]
    #[serde(rename = "auth_msg")]
    Msg {
        #[serde(with = "serde_bytes")]
        encrypted_payload: Vec<u8>,
    },
}

/// Represents authentication messages that act as initiators such as providing
/// a challenge, verifying information, presenting information, or highlighting an error
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AuthRequest {
    /// Represents a challenge comprising a series of questions to be presented
    Challenge {
        questions: Vec<AuthQuestion>,
        options: HashMap<String, String>,
    },

    /// Represents an ask to verify some information
    Verify { kind: AuthVerifyKind, text: String },

    /// Represents some information to be presented
    Info { text: String },

    /// Represents some error that occurred
    Error { kind: AuthErrorKind, text: String },
}

/// Represents authentication messages that are responses to auth requests such
/// as answers to challenges or verifying information
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AuthResponse {
    /// Represents the answers to a previously-asked challenge
    Challenge { answers: Vec<String> },

    /// Represents the answer to a previously-asked verify
    Verify { valid: bool },
}

/// Represents the type of verification being requested
#[derive(Copy, Clone, Debug, Display, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AuthVerifyKind {
    /// An ask to verify the host such as with SSH
    #[display(fmt = "host")]
    Host,
}

/// Represents a single question in a challenge
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthQuestion {
    /// The text of the question
    pub text: String,

    /// Any options information specific to a particular auth domain
    /// such as including a username and instructions for SSH authentication
    pub options: HashMap<String, String>,
}

impl AuthQuestion {
    /// Creates a new question without any options data
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            options: HashMap::new(),
        }
    }
}

/// Represents the type of error encountered during authentication
#[derive(Copy, Clone, Debug, Display, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthErrorKind {
    /// When the answer(s) to a challenge do not pass authentication
    FailedChallenge,

    /// When verification during authentication fails
    /// (e.g. a host is not allowed or blocked)
    FailedVerification,

    /// When the error is unknown
    Unknown,
}
