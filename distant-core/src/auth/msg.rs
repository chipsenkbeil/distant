use std::collections::HashMap;

use derive_more::{Display, Error, From};
use serde::{Deserialize, Serialize};

/// Represents messages from an authenticator that act as initiators such as providing
/// a challenge, verifying information, presenting information, or highlighting an error
#[derive(Clone, Debug, From, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum Authentication {
    /// Indicates the beginning of authentication, providing available methods
    #[serde(rename = "auth_initialization")]
    Initialization(Initialization),

    /// Indicates that authentication is starting for the specific `method`
    #[serde(rename = "auth_start_method")]
    StartMethod(StartMethod),

    /// Issues a challenge to be answered
    #[serde(rename = "auth_challenge")]
    Challenge(Challenge),

    /// Requests verification of some text
    #[serde(rename = "auth_verification")]
    Verification(Verification),

    /// Reports some information associated with authentication
    #[serde(rename = "auth_info")]
    Info(Info),

    /// Reports an error occurrred during authentication
    #[serde(rename = "auth_error")]
    Error(Error),

    /// Indicates that the authentication of all methods is finished
    #[serde(rename = "auth_finished")]
    Finished,
}

/// Represents the beginning of the authentication procedure
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Initialization {
    /// Available methods to use for authentication
    pub methods: Vec<String>,
}

/// Represents the start of authentication for some method
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct StartMethod {
    pub method: String,
}

/// Represents a challenge comprising a series of questions to be presented
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Challenge {
    pub questions: Vec<Question>,
    pub options: HashMap<String, String>,
}

/// Represents an ask to verify some information
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Verification {
    pub kind: VerificationKind,
    pub text: String,
}

/// Represents some information to be presented related to authentication
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Info {
    pub text: String,
}

/// Represents authentication messages that are responses to authenticator requests such
/// as answers to challenges or verifying information
#[derive(Clone, Debug, From, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum AuthenticationResponse {
    /// Contains response to initialization, providing details about which methods to use
    #[serde(rename = "auth_initialization_response")]
    Initialization(InitializationResponse),

    /// Contains answers to challenge request
    #[serde(rename = "auth_challenge_response")]
    Challenge(ChallengeResponse),

    /// Contains response to a verification request
    #[serde(rename = "auth_verification_response")]
    Verification(VerificationResponse),
}

/// Represents a response to initialization to specify which authentication methods to pursue
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InitializationResponse {
    /// Methods to use (in order as provided)
    pub methods: Vec<String>,
}

/// Represents the answers to a previously-asked challenge associated with authentication
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChallengeResponse {
    /// Answers to challenge questions (in order relative to questions)
    pub answers: Vec<String>,
}

/// Represents the answer to a previously-asked verification associated with authentication
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationResponse {
    /// Whether or not the verification was deemed valid
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

impl VerificationKind {
    /// Returns all variants except "unknown"
    pub const fn known_variants() -> &'static [Self] {
        &[Self::Host]
    }
}

/// Represents a single question in a challenge associated with authentication
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    /// Label associated with the question for more programmatic usage
    pub label: String,

    /// The text of the question (used for display purposes)
    pub text: String,

    /// Any options information specific to a particular auth domain
    /// such as including a username and instructions for SSH authentication
    pub options: HashMap<String, String>,
}

impl Question {
    /// Creates a new question without any options data using `text` for both label and text
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();

        Self {
            label: text.clone(),
            text,
            options: HashMap::new(),
        }
    }
}

/// Represents some error that occurred during authentication
#[derive(Clone, Debug, Display, Error, PartialEq, Eq, Serialize, Deserialize)]
#[display(fmt = "{kind}: {text}")]
pub struct Error {
    /// Represents the kind of error
    pub kind: ErrorKind,

    /// Description of the error
    pub text: String,
}

impl Error {
    /// Creates a fatal error
    pub fn fatal(text: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Fatal,
            text: text.into(),
        }
    }

    /// Creates a non-fatal error
    pub fn non_fatal(text: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Error,
            text: text.into(),
        }
    }

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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use test_log::test;

    use super::*;

    // ---------------------------------------------------------------
    // Authentication serde round-trips
    // ---------------------------------------------------------------

    #[test]
    fn authentication_initialization_serde_round_trip() {
        let msg = Authentication::Initialization(Initialization {
            methods: vec!["static_key".to_string(), "none".to_string()],
        });
        let json = serde_json::to_string(&msg).unwrap();
        let restored: Authentication = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, restored);
    }

    #[test]
    fn authentication_initialization_json_tag() {
        let msg = Authentication::Initialization(Initialization {
            methods: vec!["static_key".to_string()],
        });
        let val: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(val["type"], "auth_initialization");
    }

    #[test]
    fn authentication_start_method_serde_round_trip() {
        let msg = Authentication::StartMethod(StartMethod {
            method: "static_key".to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        let restored: Authentication = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, restored);
    }

    #[test]
    fn authentication_start_method_json_tag() {
        let msg = Authentication::StartMethod(StartMethod {
            method: "static_key".to_string(),
        });
        let val: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(val["type"], "auth_start_method");
    }

    #[test]
    fn authentication_challenge_serde_round_trip() {
        let mut options = HashMap::new();
        options.insert("key".to_string(), "value".to_string());
        let msg = Authentication::Challenge(Challenge {
            questions: vec![Question::new("Enter password")],
            options,
        });
        let json = serde_json::to_string(&msg).unwrap();
        let restored: Authentication = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, restored);
    }

    #[test]
    fn authentication_challenge_json_tag() {
        let msg = Authentication::Challenge(Challenge {
            questions: vec![],
            options: HashMap::new(),
        });
        let val: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(val["type"], "auth_challenge");
    }

    #[test]
    fn authentication_challenge_empty_questions_serde_round_trip() {
        let msg = Authentication::Challenge(Challenge {
            questions: vec![],
            options: HashMap::new(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        let restored: Authentication = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, restored);
    }

    #[test]
    fn authentication_verification_serde_round_trip() {
        let msg = Authentication::Verification(Verification {
            kind: VerificationKind::Host,
            text: "fingerprint abc123".to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        let restored: Authentication = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, restored);
    }

    #[test]
    fn authentication_verification_json_tag() {
        let msg = Authentication::Verification(Verification {
            kind: VerificationKind::Host,
            text: "fp".to_string(),
        });
        let val: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(val["type"], "auth_verification");
    }

    #[test]
    fn authentication_info_serde_round_trip() {
        let msg = Authentication::Info(Info {
            text: "some info".to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        let restored: Authentication = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, restored);
    }

    #[test]
    fn authentication_info_json_tag() {
        let msg = Authentication::Info(Info {
            text: "info".to_string(),
        });
        let val: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(val["type"], "auth_info");
    }

    #[test]
    fn authentication_error_serde_round_trip() {
        let msg = Authentication::Error(Error::fatal("something failed"));
        let json = serde_json::to_string(&msg).unwrap();
        let restored: Authentication = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, restored);
    }

    #[test]
    fn authentication_error_json_tag() {
        let msg = Authentication::Error(Error::non_fatal("oops"));
        let val: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(val["type"], "auth_error");
    }

    #[test]
    fn authentication_finished_serde_round_trip() {
        let msg = Authentication::Finished;
        let json = serde_json::to_string(&msg).unwrap();
        let restored: Authentication = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, restored);
    }

    #[test]
    fn authentication_finished_json_tag() {
        let msg = Authentication::Finished;
        let val: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(val["type"], "auth_finished");
    }

    // ---------------------------------------------------------------
    // AuthenticationResponse serde round-trips
    // ---------------------------------------------------------------

    #[test]
    fn authentication_response_initialization_serde_round_trip() {
        let msg = AuthenticationResponse::Initialization(InitializationResponse {
            methods: vec!["static_key".to_string()],
        });
        let json = serde_json::to_string(&msg).unwrap();
        let restored: AuthenticationResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, restored);
    }

    #[test]
    fn authentication_response_initialization_json_tag() {
        let msg =
            AuthenticationResponse::Initialization(InitializationResponse { methods: vec![] });
        let val: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(val["type"], "auth_initialization_response");
    }

    #[test]
    fn authentication_response_challenge_serde_round_trip() {
        let msg = AuthenticationResponse::Challenge(ChallengeResponse {
            answers: vec!["answer1".to_string(), "answer2".to_string()],
        });
        let json = serde_json::to_string(&msg).unwrap();
        let restored: AuthenticationResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, restored);
    }

    #[test]
    fn authentication_response_challenge_json_tag() {
        let msg = AuthenticationResponse::Challenge(ChallengeResponse { answers: vec![] });
        let val: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(val["type"], "auth_challenge_response");
    }

    #[test]
    fn authentication_response_challenge_empty_answers_serde_round_trip() {
        let msg = AuthenticationResponse::Challenge(ChallengeResponse { answers: vec![] });
        let json = serde_json::to_string(&msg).unwrap();
        let restored: AuthenticationResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, restored);
    }

    #[test]
    fn authentication_response_verification_serde_round_trip() {
        let msg = AuthenticationResponse::Verification(VerificationResponse { valid: true });
        let json = serde_json::to_string(&msg).unwrap();
        let restored: AuthenticationResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, restored);
    }

    #[test]
    fn authentication_response_verification_false_serde_round_trip() {
        let msg = AuthenticationResponse::Verification(VerificationResponse { valid: false });
        let json = serde_json::to_string(&msg).unwrap();
        let restored: AuthenticationResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, restored);
    }

    #[test]
    fn authentication_response_verification_json_tag() {
        let msg = AuthenticationResponse::Verification(VerificationResponse { valid: true });
        let val: serde_json::Value = serde_json::to_value(&msg).unwrap();
        assert_eq!(val["type"], "auth_verification_response");
    }

    // ---------------------------------------------------------------
    // VerificationKind serde and known_variants
    // ---------------------------------------------------------------

    #[test]
    fn verification_kind_host_serde_round_trip() {
        let kind = VerificationKind::Host;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, "\"host\"");
        let restored: VerificationKind = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, VerificationKind::Host);
    }

    #[test]
    fn verification_kind_unknown_deserializes_from_unrecognized_value() {
        let kind: VerificationKind = serde_json::from_str("\"something_new\"").unwrap();
        assert_eq!(kind, VerificationKind::Unknown);
    }

    #[test]
    fn verification_kind_unknown_serde_round_trip() {
        let kind = VerificationKind::Unknown;
        let json = serde_json::to_string(&kind).unwrap();
        assert_eq!(json, "\"unknown\"");
        let restored: VerificationKind = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, VerificationKind::Unknown);
    }

    #[test]
    fn verification_kind_host_display() {
        assert_eq!(format!("{}", VerificationKind::Host), "host");
    }

    #[test]
    fn verification_kind_unknown_display() {
        assert_eq!(format!("{}", VerificationKind::Unknown), "unknown");
    }

    #[test]
    fn verification_kind_known_variants_returns_only_host() {
        let variants = VerificationKind::known_variants();
        assert_eq!(variants, &[VerificationKind::Host]);
    }

    // ---------------------------------------------------------------
    // Question::new
    // ---------------------------------------------------------------

    #[test]
    fn question_new_sets_label_equal_to_text() {
        let q = Question::new("What is the password?");
        assert_eq!(q.label, "What is the password?");
        assert_eq!(q.text, "What is the password?");
    }

    #[test]
    fn question_new_has_empty_options() {
        let q = Question::new("prompt");
        assert!(q.options.is_empty());
    }

    #[test]
    fn question_new_accepts_string() {
        let q = Question::new(String::from("owned"));
        assert_eq!(q.text, "owned");
        assert_eq!(q.label, "owned");
    }

    #[test]
    fn question_serde_round_trip() {
        let mut options = HashMap::new();
        options.insert("username".to_string(), "admin".to_string());
        let q = Question {
            label: "password".to_string(),
            text: "Enter password".to_string(),
            options,
        };
        let json = serde_json::to_string(&q).unwrap();
        let restored: Question = serde_json::from_str(&json).unwrap();
        assert_eq!(q, restored);
    }

    // ---------------------------------------------------------------
    // Error constructors and methods
    // ---------------------------------------------------------------

    #[test]
    fn error_fatal_creates_fatal_error() {
        let err = Error::fatal("connection lost");
        assert_eq!(err.kind, ErrorKind::Fatal);
        assert_eq!(err.text, "connection lost");
    }

    #[test]
    fn error_non_fatal_creates_error_kind() {
        let err = Error::non_fatal("retry later");
        assert_eq!(err.kind, ErrorKind::Error);
        assert_eq!(err.text, "retry later");
    }

    #[test]
    fn error_fatal_accepts_string() {
        let err = Error::fatal(String::from("owned message"));
        assert_eq!(err.text, "owned message");
    }

    #[test]
    fn error_non_fatal_accepts_string() {
        let err = Error::non_fatal(String::from("owned"));
        assert_eq!(err.text, "owned");
    }

    #[test]
    fn error_is_fatal_returns_true_for_fatal() {
        let err = Error::fatal("fatal error");
        assert!(err.is_fatal());
    }

    #[test]
    fn error_is_fatal_returns_false_for_non_fatal() {
        let err = Error::non_fatal("non-fatal error");
        assert!(!err.is_fatal());
    }

    #[test]
    fn error_into_io_permission_denied_produces_correct_error_kind() {
        let err = Error::fatal("access denied");
        let io_err = err.into_io_permission_denied();
        assert_eq!(io_err.kind(), std::io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn error_into_io_permission_denied_preserves_message() {
        let err = Error::fatal("access denied");
        let io_err = err.into_io_permission_denied();
        assert!(io_err.to_string().contains("access denied"));
    }

    #[test]
    fn error_into_io_permission_denied_non_fatal() {
        let err = Error::non_fatal("try again");
        let io_err = err.into_io_permission_denied();
        assert_eq!(io_err.kind(), std::io::ErrorKind::PermissionDenied);
        assert!(io_err.to_string().contains("try again"));
    }

    // ---------------------------------------------------------------
    // ErrorKind::is_fatal
    // ---------------------------------------------------------------

    #[test]
    fn error_kind_fatal_is_fatal() {
        assert!(ErrorKind::Fatal.is_fatal());
    }

    #[test]
    fn error_kind_error_is_not_fatal() {
        assert!(!ErrorKind::Error.is_fatal());
    }

    #[test]
    fn error_kind_fatal_serde_round_trip() {
        let json = serde_json::to_string(&ErrorKind::Fatal).unwrap();
        let restored: ErrorKind = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, ErrorKind::Fatal);
    }

    #[test]
    fn error_kind_error_serde_round_trip() {
        let json = serde_json::to_string(&ErrorKind::Error).unwrap();
        let restored: ErrorKind = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, ErrorKind::Error);
    }

    #[test]
    fn error_kind_fatal_serializes_to_snake_case() {
        let json = serde_json::to_string(&ErrorKind::Fatal).unwrap();
        assert_eq!(json, "\"fatal\"");
    }

    #[test]
    fn error_kind_error_serializes_to_snake_case() {
        let json = serde_json::to_string(&ErrorKind::Error).unwrap();
        assert_eq!(json, "\"error\"");
    }

    // ---------------------------------------------------------------
    // Error Display
    // ---------------------------------------------------------------

    #[test]
    fn error_display_fatal() {
        let err = Error::fatal("something broke");
        assert_eq!(format!("{err}"), "Fatal: something broke");
    }

    #[test]
    fn error_display_non_fatal() {
        let err = Error::non_fatal("something went wrong");
        assert_eq!(format!("{err}"), "Error: something went wrong");
    }

    #[test]
    fn error_display_empty_text() {
        let err = Error::fatal("");
        assert_eq!(format!("{err}"), "Fatal: ");
    }

    // ---------------------------------------------------------------
    // From impls (derive_more::From) for Authentication
    // ---------------------------------------------------------------

    #[test]
    fn authentication_from_initialization() {
        let init = Initialization {
            methods: vec!["test".to_string()],
        };
        let msg: Authentication = init.clone().into();
        assert_eq!(msg, Authentication::Initialization(init));
    }

    #[test]
    fn authentication_from_start_method() {
        let sm = StartMethod {
            method: "test".to_string(),
        };
        let msg: Authentication = sm.clone().into();
        assert_eq!(msg, Authentication::StartMethod(sm));
    }

    #[test]
    fn authentication_from_challenge() {
        let ch = Challenge {
            questions: vec![Question::new("q1")],
            options: HashMap::new(),
        };
        let msg: Authentication = ch.clone().into();
        assert_eq!(msg, Authentication::Challenge(ch));
    }

    #[test]
    fn authentication_from_verification() {
        let v = Verification {
            kind: VerificationKind::Host,
            text: "text".to_string(),
        };
        let msg: Authentication = v.clone().into();
        assert_eq!(msg, Authentication::Verification(v));
    }

    #[test]
    fn authentication_from_info() {
        let info = Info {
            text: "info".to_string(),
        };
        let msg: Authentication = info.clone().into();
        assert_eq!(msg, Authentication::Info(info));
    }

    #[test]
    fn authentication_from_error() {
        let err = Error::fatal("err");
        let msg: Authentication = err.clone().into();
        assert_eq!(msg, Authentication::Error(err));
    }

    // ---------------------------------------------------------------
    // From impls (derive_more::From) for AuthenticationResponse
    // ---------------------------------------------------------------

    #[test]
    fn authentication_response_from_initialization_response() {
        let init = InitializationResponse {
            methods: vec!["test".to_string()],
        };
        let msg: AuthenticationResponse = init.clone().into();
        assert_eq!(msg, AuthenticationResponse::Initialization(init));
    }

    #[test]
    fn authentication_response_from_challenge_response() {
        let ch = ChallengeResponse {
            answers: vec!["a".to_string()],
        };
        let msg: AuthenticationResponse = ch.clone().into();
        assert_eq!(msg, AuthenticationResponse::Challenge(ch));
    }

    #[test]
    fn authentication_response_from_verification_response() {
        let v = VerificationResponse { valid: true };
        let msg: AuthenticationResponse = v.clone().into();
        assert_eq!(msg, AuthenticationResponse::Verification(v));
    }

    // ---------------------------------------------------------------
    // Deserialization from raw JSON (verifies tag + field names)
    // ---------------------------------------------------------------

    #[test]
    fn authentication_finished_deserializes_from_raw_json() {
        let json = r#"{"type":"auth_finished"}"#;
        let msg: Authentication = serde_json::from_str(json).unwrap();
        assert_eq!(msg, Authentication::Finished);
    }

    #[test]
    fn authentication_initialization_deserializes_from_raw_json() {
        let json = r#"{"type":"auth_initialization","methods":["static_key"]}"#;
        let msg: Authentication = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            Authentication::Initialization(Initialization {
                methods: vec!["static_key".to_string()],
            })
        );
    }

    #[test]
    fn authentication_start_method_deserializes_from_raw_json() {
        let json = r#"{"type":"auth_start_method","method":"static_key"}"#;
        let msg: Authentication = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            Authentication::StartMethod(StartMethod {
                method: "static_key".to_string(),
            })
        );
    }

    #[test]
    fn authentication_info_deserializes_from_raw_json() {
        let json = r#"{"type":"auth_info","text":"hello"}"#;
        let msg: Authentication = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            Authentication::Info(Info {
                text: "hello".to_string(),
            })
        );
    }

    #[test]
    fn authentication_error_deserializes_from_raw_json() {
        let json = r#"{"type":"auth_error","kind":"fatal","text":"bad"}"#;
        let msg: Authentication = serde_json::from_str(json).unwrap();
        assert_eq!(msg, Authentication::Error(Error::fatal("bad")));
    }

    #[test]
    fn authentication_response_initialization_deserializes_from_raw_json() {
        let json = r#"{"type":"auth_initialization_response","methods":["none"]}"#;
        let msg: AuthenticationResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            AuthenticationResponse::Initialization(InitializationResponse {
                methods: vec!["none".to_string()],
            })
        );
    }

    #[test]
    fn authentication_response_challenge_deserializes_from_raw_json() {
        let json = r#"{"type":"auth_challenge_response","answers":["pw"]}"#;
        let msg: AuthenticationResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            AuthenticationResponse::Challenge(ChallengeResponse {
                answers: vec!["pw".to_string()],
            })
        );
    }

    #[test]
    fn authentication_response_verification_deserializes_from_raw_json() {
        let json = r#"{"type":"auth_verification_response","valid":false}"#;
        let msg: AuthenticationResponse = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            AuthenticationResponse::Verification(VerificationResponse { valid: false })
        );
    }

    // ---------------------------------------------------------------
    // Error serde round-trip (struct directly)
    // ---------------------------------------------------------------

    #[test]
    fn error_struct_serde_round_trip_fatal() {
        let err = Error::fatal("fatal error");
        let json = serde_json::to_string(&err).unwrap();
        let restored: Error = serde_json::from_str(&json).unwrap();
        assert_eq!(err, restored);
    }

    #[test]
    fn error_struct_serde_round_trip_non_fatal() {
        let err = Error::non_fatal("non-fatal");
        let json = serde_json::to_string(&err).unwrap();
        let restored: Error = serde_json::from_str(&json).unwrap();
        assert_eq!(err, restored);
    }

    // ---------------------------------------------------------------
    // Struct-level serde round-trips for standalone structs
    // ---------------------------------------------------------------

    #[test]
    fn initialization_serde_round_trip() {
        let init = Initialization {
            methods: vec!["a".to_string(), "b".to_string()],
        };
        let json = serde_json::to_string(&init).unwrap();
        let restored: Initialization = serde_json::from_str(&json).unwrap();
        assert_eq!(init, restored);
    }

    #[test]
    fn start_method_serde_round_trip() {
        let sm = StartMethod {
            method: "static_key".to_string(),
        };
        let json = serde_json::to_string(&sm).unwrap();
        let restored: StartMethod = serde_json::from_str(&json).unwrap();
        assert_eq!(sm, restored);
    }

    #[test]
    fn challenge_serde_round_trip() {
        let mut options = HashMap::new();
        options.insert("k".to_string(), "v".to_string());
        let ch = Challenge {
            questions: vec![Question::new("q")],
            options,
        };
        let json = serde_json::to_string(&ch).unwrap();
        let restored: Challenge = serde_json::from_str(&json).unwrap();
        assert_eq!(ch, restored);
    }

    #[test]
    fn verification_serde_round_trip() {
        let v = Verification {
            kind: VerificationKind::Host,
            text: "fp".to_string(),
        };
        let json = serde_json::to_string(&v).unwrap();
        let restored: Verification = serde_json::from_str(&json).unwrap();
        assert_eq!(v, restored);
    }

    #[test]
    fn info_serde_round_trip() {
        let info = Info {
            text: "hello".to_string(),
        };
        let json = serde_json::to_string(&info).unwrap();
        let restored: Info = serde_json::from_str(&json).unwrap();
        assert_eq!(info, restored);
    }

    #[test]
    fn initialization_response_serde_round_trip() {
        let ir = InitializationResponse {
            methods: vec!["m".to_string()],
        };
        let json = serde_json::to_string(&ir).unwrap();
        let restored: InitializationResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(ir, restored);
    }

    #[test]
    fn challenge_response_serde_round_trip() {
        let cr = ChallengeResponse {
            answers: vec!["a".to_string()],
        };
        let json = serde_json::to_string(&cr).unwrap();
        let restored: ChallengeResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(cr, restored);
    }

    #[test]
    fn verification_response_serde_round_trip() {
        let vr = VerificationResponse { valid: true };
        let json = serde_json::to_string(&vr).unwrap();
        let restored: VerificationResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(vr, restored);
    }

    // ---------------------------------------------------------------
    // Verification with Unknown kind through enclosing enum
    // ---------------------------------------------------------------

    #[test]
    fn authentication_verification_with_unknown_kind_serde_round_trip() {
        let msg = Authentication::Verification(Verification {
            kind: VerificationKind::Unknown,
            text: "unknown thing".to_string(),
        });
        let json = serde_json::to_string(&msg).unwrap();
        let restored: Authentication = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, restored);
    }

    #[test]
    fn authentication_verification_unrecognized_kind_deserializes_as_unknown() {
        let json = r#"{"type":"auth_verification","kind":"some_future_variant","text":"test"}"#;
        let msg: Authentication = serde_json::from_str(json).unwrap();
        assert_eq!(
            msg,
            Authentication::Verification(Verification {
                kind: VerificationKind::Unknown,
                text: "test".to_string(),
            })
        );
    }

    // ---------------------------------------------------------------
    // Challenge with multiple questions and options
    // ---------------------------------------------------------------

    #[test]
    fn authentication_challenge_multiple_questions_serde_round_trip() {
        let mut opts = HashMap::new();
        opts.insert("instructions".to_string(), "Please answer".to_string());
        let msg = Authentication::Challenge(Challenge {
            questions: vec![
                Question {
                    label: "username".to_string(),
                    text: "Username:".to_string(),
                    options: HashMap::new(),
                },
                Question {
                    label: "password".to_string(),
                    text: "Password:".to_string(),
                    options: {
                        let mut m = HashMap::new();
                        m.insert("echo".to_string(), "false".to_string());
                        m
                    },
                },
            ],
            options: opts,
        });
        let json = serde_json::to_string(&msg).unwrap();
        let restored: Authentication = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, restored);
    }
}
