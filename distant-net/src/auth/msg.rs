use derive_more::{Display, Error, From};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents messages from an authenticator that act as initiators such as providing
/// a challenge, verifying information, presenting information, or highlighting an error
#[derive(Clone, Debug, From, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Authentication {
    /// Indicates the beginning of authentication, providing available methods
    Initialization(Initialization),

    /// Indicates that authentication is starting for the specific `method`
    StartMethod(StartMethod),

    /// Issues a challenge to be answered
    Challenge(Challenge),

    /// Requests verification of some text
    Verification(Verification),

    /// Reports some information associated with authentication
    Info(Info),

    /// Reports an error occurrred during authentication
    Error(Error),

    /// Indicates that the authentication of all methods is finished
    Finished,
}

/// Represents the beginning of the authentication procedure
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Initialization {
    /// Available methods to use for authentication
    pub methods: Vec<Method>,
}

/// Represents the start of authentication for some method
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StartMethod {
    pub method: Method,
}

/// Represents the type of authentication method to use by the authenticator
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Method {
    /// Indicates that a static key is being used for authentication
    StaticKey,

    /// Indicates that re-authentication is being employed (using specialized key)
    Reauthentication,

    /// When the method is unknown (happens when other side is unaware of the method)
    #[serde(other)]
    Unknown,
}

/// Represents a challenge comprising a series of questions to be presented
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Challenge {
    pub questions: Vec<Question>,
    pub options: HashMap<String, String>,
}

/// Represents an ask to verify some information
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Verification {
    pub kind: VerificationKind,
    pub text: String,
}

/// Represents some information to be presented related to authentication
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Info {
    pub text: String,
}

/// Represents authentication messages that are responses to authenticator requests such
/// as answers to challenges or verifying information
#[derive(Clone, Debug, From, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AuthenticationResponse {
    /// Contains response to initialization, providing details about which methods to use
    Initialization(InitializationResponse),

    /// Contains answers to challenge request
    Challenge(ChallengeResponse),

    /// Contains response to a verification request
    Verification(VerificationResponse),
}

/// Represents a response to initialization to specify which authentication methods to pursue
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InitializationResponse {
    pub methods: Vec<Method>,
}

/// Represents the answers to a previously-asked challenge associated with authentication
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChallengeResponse {
    pub answers: Vec<String>,
}

/// Represents the answer to a previously-asked verification associated with authentication
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerificationResponse {
    pub valid: bool,
}

/// Represents the type of verification being requested
#[derive(Copy, Clone, Debug, Display, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationKind {
    /// An ask to verify the host such as with SSH
    #[display(fmt = "host")]
    Host,

    /// When the verification is unknown (happens when other side is unaware of the kind)
    #[display(fmt = "unknown")]
    #[serde(other)]
    Unknown,
}

/// Represents a single question in a challenge associated with authentication
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    /// The text of the question
    pub text: String,

    /// Any options information specific to a particular auth domain
    /// such as including a username and instructions for SSH authentication
    pub options: HashMap<String, String>,
}

impl Question {
    /// Creates a new question without any options data
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            options: HashMap::new(),
        }
    }
}

/// Represents some error that occurred during authentication
#[derive(Clone, Debug, Display, Error, Serialize, Deserialize)]
#[display(fmt = "{}: {}", kind, text)]
pub struct Error {
    /// Represents the kind of error
    pub kind: ErrorKind,

    /// Description of the error
    pub text: String,
}

impl Error {
    /// Returns true if error represents a fatal error, meaning that there is no recovery possible
    /// from this error
    pub fn is_fatal(&self) -> bool {
        self.kind.is_fatal()
    }

    /// Converts the error into a [`std::io::Error`] representing permission denied
    pub fn into_io_permission_denied(self) -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::PermissionDenied, self)
    }
}

/// Represents the type of error encountered during authentication
#[derive(Copy, Clone, Debug, Display, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorKind {
    /// Error is unrecoverable
    Fatal,

    /// Error is recoverable
    Error,
}

impl ErrorKind {
    /// Returns true if error kind represents a fatal error, meaning that there is no recovery
    /// possible from this error
    pub fn is_fatal(self) -> bool {
        matches!(self, Self::Fatal)
    }
}
