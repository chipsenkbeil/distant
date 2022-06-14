use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents authentication messages that can be sent over the wire
#[derive(Debug, Serialize, Deserialize)]
pub enum Auth {
    /// Represents a request to perform an authentication handshake,
    /// providing the public key and salt from one side in order to
    /// derive the shared key
    Handshake(PubKey, Salt),

    /// Represents the bytes of an encrypted message
    ///
    /// Underneath, will be one of either [`AuthRequest`] or [`AuthResponse`]
    Msg(Vec<u8>),
}

/// Represents authentication messages that act as initiators such as providing
/// a challenge, verifying information, presenting information, or highlighting an error
#[derive(Debug, Serialize, Deserialize)]
pub enum AuthRequest {
    Challenge(Questions, AuthExtra),
    Verify(AuthVerifyKind, String),
    Info(String),
    Error(AuthErrorKind, String),
}

/// Represents authentication messages that are responses to auth requests such
/// as answers to challenges or verifying information
#[derive(Debug, Serialize, Deserialize)]
pub enum AuthResponse {
    Challenge(Answers),
    Yes,
    No,
}

/// Represents the type of verification being requested
#[derive(Copy, Clone, Debug, ParialEq, Eq, Serialize, Deserialize)]
pub enum AuthVerifyKind {
    /// An ask to verify the host such as with SSH
    Host,
}

/// Format for extra data included in some auth message
pub type AuthExtra = HashMap<String, String>;

/// Represents a collection of questions
#[derive(Debug, Serialize, Deserialize)]
pub struct Questions(Vec<Question>);

/// Represents a single question in a challenge
#[derive(Debug, Serialize, Deserialize)]
pub struct Question {
    /// The text of the question
    text: String,

    /// Any extra information specific to a particular auth domain
    /// such as including a username and instructions for SSH authentication
    extra: AuthExtra,
}

/// Represents a collection of answers
#[derive(Debug, Serialize, Deserialize)]
pub struct Answers(Vec<String>);
