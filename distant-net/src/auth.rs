use p256::EncodedPoint;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

mod client;
pub use client::*;

mod handshake;
pub use handshake::*;

mod server;
pub use server::*;

/// Represents authentication messages that can be sent over the wire
#[derive(Debug, Serialize, Deserialize)]
pub enum Auth {
    /// Represents a request to perform an authentication handshake,
    /// providing the public key and salt from one side in order to
    /// derive the shared key
    Handshake {
        /// Bytes of the public key
        public_key: EncodedPoint,

        /// Randomly generated salt
        salt: Salt,
    },

    /// Represents the bytes of an encrypted message
    ///
    /// Underneath, will be one of either [`AuthRequest`] or [`AuthResponse`]
    Msg { encrypted_payload: Vec<u8> },
}

/// Represents authentication messages that act as initiators such as providing
/// a challenge, verifying information, presenting information, or highlighting an error
#[derive(Debug, Serialize, Deserialize)]
pub enum AuthRequest {
    /// Represents a challenge comprising a series of questions to be presented
    Challenge {
        questions: Vec<Question>,
        extra: HashMap<String, String>,
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
#[derive(Debug, Serialize, Deserialize)]
pub enum AuthResponse {
    /// Represents the answers to a previously-asked challenge
    Challenge { answers: Vec<String> },

    /// Represents the answer to a previously-asked verify
    Verify { valid: bool },
}

/// Represents the type of verification being requested
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthVerifyKind {
    /// An ask to verify the host such as with SSH
    Host,
}

/// Represents a single question in a challenge
#[derive(Debug, Serialize, Deserialize)]
pub struct Question {
    /// The text of the question
    pub text: String,

    /// Any extra information specific to a particular auth domain
    /// such as including a username and instructions for SSH authentication
    pub extra: HashMap<String, String>,
}

impl Question {
    /// Creates a new question without any extra data
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            extra: HashMap::new(),
        }
    }
}

/// Represents the type of error encountered during authentication
#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub enum AuthErrorKind {
    /// When the answer(s) to a challenge do not pass authentication
    FailedChallenge,

    /// When verification during authentication fails
    /// (e.g. a host is not allowed or blocked)
    FailedVerification,

    /// When the error is unknown
    Unknown,
}
