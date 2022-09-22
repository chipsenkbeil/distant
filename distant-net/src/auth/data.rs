use async_trait::async_trait;
use derive_more::Display;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io};

/// Interface for a handler of authentication requests
#[async_trait]
pub trait AuthHandler {
    /// Callback when a challenge is received, returning answers to the given questions.
    async fn on_challenge(
        &mut self,
        questions: Vec<AuthQuestion>,
        options: HashMap<String, String>,
    ) -> io::Result<Vec<String>>;

    /// Callback when a verification request is received, returning true if approvided or false if
    /// unapproved.
    async fn on_verify(&mut self, kind: AuthVerifyKind, text: String) -> io::Result<bool>;

    /// Callback when authentication is finished and no more requests will be received
    async fn on_done(&mut self) -> io::Result<()> {
        Ok(())
    }

    /// Callback when information is received. To fail, return an error from this function.
    #[allow(unused_variables)]
    async fn on_info(&mut self, text: String) -> io::Result<()> {
        Ok(())
    }

    /// Callback when an error is received. To fail, return an error from this function.
    async fn on_error(&mut self, kind: AuthErrorKind, text: String) -> io::Result<()> {
        Err(match kind {
            AuthErrorKind::FailedChallenge => io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("Failed challenge: {text}"),
            ),
            AuthErrorKind::FailedVerification => io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("Failed verification: {text}"),
            ),
            AuthErrorKind::Unknown => {
                io::Error::new(io::ErrorKind::Other, format!("Unknown error: {text}"))
            }
        })
    }
}

/// Represents authentication messages that act as initiators such as providing
/// a challenge, verifying information, presenting information, or highlighting an error
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AuthRequest {
    Challenge(AuthChallengeRequest),
    Verify(AuthVerifyRequest),
    Info(AuthInfo),
    Error(AuthError),
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
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AuthResponse {
    Challenge(AuthChallengeResponse),
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
