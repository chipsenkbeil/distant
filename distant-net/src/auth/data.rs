use derive_more::{Display, From};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents authentication messages that act as initiators such as providing
/// a challenge, verifying information, presenting information, or highlighting an error
#[derive(Clone, Debug, From, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AuthRequest {
    /// Issues a challenge to be answered
    Challenge(AuthChallengeRequest),

    /// Requests verification of some text
    Verify(AuthVerifyRequest),

    /// Reports some information
    Info(AuthInfo),

    /// Reports an error occurrred
    Error(AuthError),

    /// Indicates that the authentication is finished
    Finished,
}

/// Represents a challenge comprising a series of questions to be presented
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthChallengeRequest {
    pub questions: Vec<AuthQuestion>,
    pub options: HashMap<String, String>,
}

/// Represents an ask to verify some information
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthVerifyRequest {
    pub kind: AuthVerifyKind,
    pub text: String,
}

/// Represents some information to be presented
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthInfo {
    pub text: String,
}

/// Represents some error that occurred
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthError {
    pub kind: AuthErrorKind,
    pub text: String,
}

/// Represents authentication messages that are responses to auth requests such
/// as answers to challenges or verifying information
#[derive(Clone, Debug, From, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AuthResponse {
    /// Contains answers to challenge request
    Challenge(AuthChallengeResponse),

    /// Contains response to a verification request
    Verify(AuthVerifyResponse),
}

/// Represents the answers to a previously-asked challenge
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthChallengeResponse {
    pub answers: Vec<String>,
}

/// Represents the answer to a previously-asked verify
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthVerifyResponse {
    pub valid: bool,
}

/// Represents the type of verification being requested
#[derive(Copy, Clone, Debug, Display, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AuthVerifyKind {
    /// An ask to verify the host such as with SSH
    #[display(fmt = "host")]
    Host,

    /// When the verification is unknown (happens when other side is unaware of the kind)
    #[display(fmt = "unknown")]
    #[serde(other)]
    Unknown,
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
