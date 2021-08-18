use crate::core::session::{Session, SessionParseError};
use derive_more::{Display, Error, From};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    fmt,
    io::{self, BufRead},
    ops::{Deref, DerefMut},
    str::FromStr,
    string::FromUtf8Error,
};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Display, Error, From)]
pub enum LspSessionError {
    /// Encountered when attempting to create a session from a request that is not initialize
    NotInitializeRequest,

    /// Encountered if missing session parameters within an initialize request
    MissingSessionParams,

    /// Encountered if session parameters are not expected types
    InvalidSessionParams,

    /// Encountered when failing to parse session
    SessionParseError(SessionParseError),
}

impl From<LspSessionError> for io::Error {
    fn from(x: LspSessionError) -> Self {
        match x {
            LspSessionError::SessionParseError(x) => x.into(),
            x => io::Error::new(io::ErrorKind::InvalidData, x),
        }
    }
}

/// Represents some data being communicated to/from an LSP consisting of a header and content part
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspData {
    /// Header-portion of some data related to LSP
    header: LspDataHeader,

    /// Content-portion of some data related to LSP
    content: LspDataContent,
}

#[derive(Debug, Display, Error, From)]
pub enum LspDataParseError {
    /// When the received content is malformed
    BadContent(LspDataContentParseError),

    /// When the received header is malformed
    BadHeader(LspDataHeaderParseError),

    /// When a header line is not terminated in \r\n
    BadHeaderTermination,

    /// When input fails to be in UTF-8 format
    BadInput(FromUtf8Error),

    /// When some unexpected I/O error encountered
    IoError(io::Error),

    /// When EOF received before data fully acquired
    UnexpectedEof,
}

impl From<LspDataParseError> for io::Error {
    fn from(x: LspDataParseError) -> Self {
        match x {
            LspDataParseError::BadContent(x) => x.into(),
            LspDataParseError::BadHeader(x) => x.into(),
            LspDataParseError::BadHeaderTermination => io::Error::new(
                io::ErrorKind::InvalidData,
                r"Received header line not terminated in \r\n",
            ),
            LspDataParseError::BadInput(x) => io::Error::new(io::ErrorKind::InvalidData, x),
            LspDataParseError::IoError(x) => x,
            LspDataParseError::UnexpectedEof => io::Error::from(io::ErrorKind::UnexpectedEof),
        }
    }
}

impl LspData {
    /// Returns a reference to the header part
    pub fn header(&self) -> &LspDataHeader {
        &self.header
    }

    /// Returns a reference to the content part
    pub fn content(&self) -> &LspDataContent {
        &self.content
    }

    /// Creates a session by inspecting the content for session parameters, removing the session
    /// parameters from the content. Will also adjust the content length header to match the
    /// new size of the content.
    pub fn take_session(&mut self) -> Result<Session, LspSessionError> {
        match self.content.take_session() {
            Ok(session) => {
                self.header.content_length = self.content.to_string().len();
                Ok(session)
            }
            Err(x) => Err(x),
        }
    }

    /// Attempts to read incoming lsp data from a buffered reader.
    ///
    /// Note that this is **blocking** while it waits on the header information!
    ///
    /// ```text
    /// Content-Length: ...\r\n
    /// Content-Type: ...\r\n
    /// \r\n
    /// {
    ///     "jsonrpc": "2.0",
    ///     ...
    /// }
    /// ```
    pub fn from_buf_reader<R: BufRead>(r: &mut R) -> Result<Self, LspDataParseError> {
        // Read in our headers first so we can figure out how much more to read
        let mut buf = String::new();
        loop {
            // Track starting position for new buffer content
            let start = buf.len();

            // Block on each line of input!
            let len = r.read_line(&mut buf)?;
            let end = start + len;

            // We shouldn't be getting end of the reader yet
            if len == 0 {
                return Err(LspDataParseError::UnexpectedEof);
            }

            let line = &buf[start..end];

            // Check if we've gotten bad data
            if !line.ends_with("\r\n") {
                return Err(LspDataParseError::BadHeaderTermination);

            // Check if we've received the header termination
            } else if line == "\r\n" {
                break;
            }
        }

        // Parse the header content so we know how much more to read
        let header = buf.parse::<LspDataHeader>()?;

        // Read remaining content
        let content = {
            let mut buf = vec![0u8; header.content_length];
            r.read_exact(&mut buf).map_err(|x| {
                if x.kind() == io::ErrorKind::UnexpectedEof {
                    LspDataParseError::UnexpectedEof
                } else {
                    LspDataParseError::IoError(x)
                }
            })?;
            String::from_utf8(buf)?.parse::<LspDataContent>()?
        };

        Ok(Self { header, content })
    }
}

impl fmt::Display for LspData {
    /// Outputs header & content in form
    ///
    /// ```text
    /// Content-Length: ...\r\n
    /// Content-Type: ...\r\n
    /// \r\n
    /// {
    ///     "jsonrpc": "2.0",
    ///     ...
    /// }
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.header.fmt(f)?;
        self.content.fmt(f)
    }
}

impl FromStr for LspData {
    type Err = LspDataParseError;

    /// Parses headers and content in the form of
    ///
    /// ```text
    /// Content-Length: ...\r\n
    /// Content-Type: ...\r\n
    /// \r\n
    /// {
    ///     "jsonrpc": "2.0",
    ///     ...
    /// }
    /// ```
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut r = io::BufReader::new(io::Cursor::new(s));
        Self::from_buf_reader(&mut r)
    }
}

/// Represents the header for LSP data
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspDataHeader {
    /// Length of content part in bytes
    pub content_length: usize,

    /// Mime type of content part, defaulting to
    /// application/vscode-jsonrpc; charset=utf-8
    pub content_type: Option<String>,
}

impl fmt::Display for LspDataHeader {
    /// Outputs header in form
    ///
    /// ```text
    /// Content-Length: ...\r\n
    /// Content-Type: ...\r\n
    /// \r\n
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Content-Length: {}\r\n", self.content_length)?;

        if let Some(ty) = self.content_type.as_ref() {
            write!(f, "Content-Type: {}\r\n", ty)?;
        }

        write!(f, "\r\n")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Display, Error, From)]
pub enum LspDataHeaderParseError {
    MissingContentLength,
    InvalidContentLength(std::num::ParseIntError),
    BadHeaderField,
}

impl From<LspDataHeaderParseError> for io::Error {
    fn from(x: LspDataHeaderParseError) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, x)
    }
}

impl FromStr for LspDataHeader {
    type Err = LspDataHeaderParseError;

    /// Parses headers in the form of
    ///
    /// ```text
    /// Content-Length: ...\r\n
    /// Content-Type: ...\r\n
    /// \r\n
    /// ```
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let lines = s.split("\r\n").map(str::trim).filter(|l| !l.is_empty());
        let mut content_length = None;
        let mut content_type = None;

        for line in lines {
            if let Some((name, value)) = line.split_once(':') {
                match name {
                    "Content-Length" => content_length = Some(value.trim().parse()?),
                    "Content-Type" => content_type = Some(value.trim().to_string()),
                    _ => return Err(LspDataHeaderParseError::BadHeaderField),
                }
            } else {
                return Err(LspDataHeaderParseError::BadHeaderField);
            }
        }

        match content_length {
            Some(content_length) => Ok(Self {
                content_length,
                content_type,
            }),
            None => Err(LspDataHeaderParseError::MissingContentLength),
        }
    }
}

/// Represents the content for LSP data
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspDataContent(Map<String, Value>);

impl LspDataContent {
    /// Creates a session by inspecting the content for session parameters, removing the session
    /// parameters from the content
    pub fn take_session(&mut self) -> Result<Session, LspSessionError> {
        // Verify that we're dealing with an initialize request
        match self.0.get("method") {
            Some(value) if value == "initialize" => {}
            _ => return Err(LspSessionError::NotInitializeRequest),
        }

        // Attempt to grab the distant initialization options
        match self.strip_session_params() {
            Some((Some(host), Some(port), Some(auth_key))) => {
                let host = host.as_str().ok_or(LspSessionError::InvalidSessionParams)?;
                let port = port.as_u64().ok_or(LspSessionError::InvalidSessionParams)?;
                let auth_key = auth_key
                    .as_str()
                    .ok_or(LspSessionError::InvalidSessionParams)?;
                Ok(format!("DISTANT DATA {} {} {}", host, port, auth_key).parse()?)
            }
            _ => Err(LspSessionError::MissingSessionParams),
        }
    }

    /// Strips the session params from the content, returning them if they exist
    ///
    /// ```json
    /// {
    ///     "params": {
    ///         "initializationOptions": {
    ///             "distant": {
    ///                 "host": "...",
    ///                 "port": ...,
    ///                 "auth_key": "..."
    ///             }
    ///         }
    ///     }
    /// }
    /// ```
    fn strip_session_params(&mut self) -> Option<(Option<Value>, Option<Value>, Option<Value>)> {
        self.0
            .get_mut("params")
            .and_then(|v| v.get_mut("initializationOptions"))
            .and_then(|v| v.as_object_mut())
            .and_then(|o| o.remove("distant"))
            .map(|mut v| {
                (
                    v.get_mut("host").map(Value::take),
                    v.get_mut("port").map(Value::take),
                    v.get_mut("auth_key").map(Value::take),
                )
            })
    }
}

impl AsRef<Map<String, Value>> for LspDataContent {
    fn as_ref(&self) -> &Map<String, Value> {
        &self.0
    }
}

impl Deref for LspDataContent {
    type Target = Map<String, Value>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for LspDataContent {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl fmt::Display for LspDataContent {
    /// Outputs content in JSON form
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}",
            serde_json::to_string_pretty(self).map_err(|_| fmt::Error)?
        )
    }
}

#[derive(Debug, Display, Error, From)]
pub struct LspDataContentParseError(serde_json::Error);

impl From<LspDataContentParseError> for io::Error {
    fn from(x: LspDataContentParseError) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, x)
    }
}

impl FromStr for LspDataContent {
    type Err = LspDataContentParseError;

    /// Parses content in JSON form
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_str(s).map_err(From::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::SecretKey;

    macro_rules! make_obj {
        ($($tail:tt)*) => {
            match serde_json::json!($($tail)*) {
                serde_json::Value::Object(x) => x,
                x => panic!("Got non-object: {:?}", x),
            }
        };
    }

    #[test]
    fn data_display_should_output_header_and_content() {
        let data = LspData {
            header: LspDataHeader {
                content_length: 123,
                content_type: Some(String::from("some content type")),
            },
            content: LspDataContent(make_obj!({"hello": "world"})),
        };

        let output = data.to_string();
        assert_eq!(
            output,
            concat!(
                "Content-Length: 123\r\n",
                "Content-Type: some content type\r\n",
                "\r\n",
                "{\n",
                "  \"hello\": \"world\"\n",
                "}",
            )
        );
    }

    #[test]
    fn data_from_buf_reader_should_be_successful_if_valid_data_received() {
        let mut input = io::Cursor::new(concat!(
            "Content-Length: 22\r\n",
            "Content-Type: some content type\r\n",
            "\r\n",
            "{\n",
            "  \"hello\": \"world\"\n",
            "}",
        ));
        let data = LspData::from_buf_reader(&mut input).unwrap();
        assert_eq!(data.header.content_length, 22);
        assert_eq!(
            data.header.content_type.as_deref(),
            Some("some content type")
        );
        assert_eq!(data.content.as_ref(), &make_obj!({ "hello": "world" }));
    }

    #[test]
    fn data_from_buf_reader_should_fail_if_reach_eof_before_received_full_data() {
        // Header doesn't finish
        let err = LspData::from_buf_reader(&mut io::Cursor::new(concat!(
            "Content-Length: 22\r\n",
            "Content-Type: some content type\r\n",
        )))
        .unwrap_err();
        assert!(matches!(err, LspDataParseError::UnexpectedEof), "{:?}", err);

        // No content after header
        let err = LspData::from_buf_reader(&mut io::Cursor::new(concat!(
            "Content-Length: 22\r\n",
            "\r\n",
        )))
        .unwrap_err();
        assert!(matches!(err, LspDataParseError::UnexpectedEof), "{:?}", err);
    }

    #[test]
    fn data_from_buf_reader_should_fail_if_missing_proper_line_termination_for_header_field() {
        let err = LspData::from_buf_reader(&mut io::Cursor::new(concat!(
            "Content-Length: 22\n",
            "\r\n",
            "{\n",
            "  \"hello\": \"world\"\n",
            "}",
        )))
        .unwrap_err();
        assert!(
            matches!(err, LspDataParseError::BadHeaderTermination),
            "{:?}",
            err
        );
    }

    #[test]
    fn data_from_buf_reader_should_fail_if_bad_header_provided() {
        // Invalid content length
        let err = LspData::from_buf_reader(&mut io::Cursor::new(concat!(
            "Content-Length: -1\r\n",
            "\r\n",
            "{\n",
            "  \"hello\": \"world\"\n",
            "}",
        )))
        .unwrap_err();
        assert!(matches!(err, LspDataParseError::BadHeader(_)), "{:?}", err);

        // Missing content length
        let err = LspData::from_buf_reader(&mut io::Cursor::new(concat!(
            "Content-Type: some content type\r\n",
            "\r\n",
            "{\n",
            "  \"hello\": \"world\"\n",
            "}",
        )))
        .unwrap_err();
        assert!(matches!(err, LspDataParseError::BadHeader(_)), "{:?}", err);
    }

    #[test]
    fn data_from_buf_reader_should_fail_if_bad_content_provided() {
        // Not full content
        let err = LspData::from_buf_reader(&mut io::Cursor::new(concat!(
            "Content-Length: 21\r\n",
            "\r\n",
            "{\n",
            "  \"hello\": \"world\"\n",
        )))
        .unwrap_err();
        assert!(matches!(err, LspDataParseError::BadContent(_)), "{:?}", err);
    }

    #[test]
    fn data_from_buf_reader_should_fail_if_non_utf8_data_encountered_for_content() {
        // Not utf-8 content
        let mut raw = b"Content-Length: 2\r\n\r\n".to_vec();
        raw.extend(vec![0, 159]);

        let err = LspData::from_buf_reader(&mut io::Cursor::new(raw)).unwrap_err();
        assert!(matches!(err, LspDataParseError::BadInput(_)), "{:?}", err);
    }

    #[test]
    fn data_take_session_should_succeed_if_valid_session_found_in_params() {
        let mut data = LspData {
            header: LspDataHeader {
                content_length: 123,
                content_type: Some(String::from("some content type")),
            },
            content: LspDataContent(make_obj!({
                "method": "initialize",
                "params": {
                    "initializationOptions": {
                        "distant": {
                            "host": "some.host",
                            "port": 22,
                            "auth_key": "abc123"
                        }
                    }
                }
            })),
        };

        let session = data.take_session().unwrap();
        assert_eq!(
            session,
            Session {
                host: String::from("some.host"),
                port: 22,
                auth_key: SecretKey::from_slice(&hex::decode(b"abc123").unwrap()).unwrap(),
            }
        );
    }

    #[test]
    fn data_take_session_should_remove_session_parameters_if_successful() {
        let mut data = LspData {
            header: LspDataHeader {
                content_length: 123,
                content_type: Some(String::from("some content type")),
            },
            content: LspDataContent(make_obj!({
                "method": "initialize",
                "params": {
                    "initializationOptions": {
                        "distant": {
                            "host": "some.host",
                            "port": 22,
                            "auth_key": "abc123"
                        }
                    }
                }
            })),
        };

        let _ = data.take_session().unwrap();
        assert_eq!(
            data.content.as_ref(),
            &make_obj!({
                "method": "initialize",
                "params": {
                    "initializationOptions": {}
                }
            })
        );
    }

    #[test]
    fn data_take_session_should_adjust_content_length_based_on_new_content_byte_length() {
        let mut data = LspData {
            header: LspDataHeader {
                content_length: 123456,
                content_type: Some(String::from("some content type")),
            },
            content: LspDataContent(make_obj!({
                "method": "initialize",
                "params": {
                    "initializationOptions": {
                        "distant": {
                            "host": "some.host",
                            "port": 22,
                            "auth_key": "abc123"
                        }
                    }
                }
            })),
        };

        let _ = data.take_session().unwrap();
        assert_eq!(data.header.content_length, data.content.to_string().len());
    }

    #[test]
    fn data_take_session_should_fail_if_path_incomplete_to_session_params() {
        let mut data = LspData {
            header: LspDataHeader {
                content_length: 123456,
                content_type: Some(String::from("some content type")),
            },
            content: LspDataContent(make_obj!({
                "method": "initialize",
                "params": {
                    "initializationOptions": {}
                }
            })),
        };

        let err = data.take_session().unwrap_err();
        assert!(
            matches!(err, LspSessionError::MissingSessionParams),
            "{:?}",
            err
        );
    }

    #[test]
    fn data_take_session_should_fail_if_missing_host_param() {
        let mut data = LspData {
            header: LspDataHeader {
                content_length: 123456,
                content_type: Some(String::from("some content type")),
            },
            content: LspDataContent(make_obj!({
                "method": "initialize",
                "params": {
                    "initializationOptions": {
                        "distant": {
                            "port": 22,
                            "auth_key": "abc123"
                        }
                    }
                }
            })),
        };

        let err = data.take_session().unwrap_err();
        assert!(
            matches!(err, LspSessionError::MissingSessionParams),
            "{:?}",
            err
        );
    }

    #[test]
    fn data_take_session_should_fail_if_host_param_is_invalid() {
        let mut data = LspData {
            header: LspDataHeader {
                content_length: 123456,
                content_type: Some(String::from("some content type")),
            },
            content: LspDataContent(make_obj!({
                "method": "initialize",
                "params": {
                    "initializationOptions": {
                        "distant": {
                            "host": 1234,
                            "port": 22,
                            "auth_key": "abc123"
                        }
                    }
                }
            })),
        };

        let err = data.take_session().unwrap_err();
        assert!(
            matches!(err, LspSessionError::InvalidSessionParams),
            "{:?}",
            err
        );
    }

    #[test]
    fn data_take_session_should_fail_if_missing_port_param() {
        let mut data = LspData {
            header: LspDataHeader {
                content_length: 123456,
                content_type: Some(String::from("some content type")),
            },
            content: LspDataContent(make_obj!({
                "method": "initialize",
                "params": {
                    "initializationOptions": {
                        "distant": {
                            "host": "some.host",
                            "auth_key": "abc123"
                        }
                    }
                }
            })),
        };

        let err = data.take_session().unwrap_err();
        assert!(
            matches!(err, LspSessionError::MissingSessionParams),
            "{:?}",
            err
        );
    }

    #[test]
    fn data_take_session_should_fail_if_port_param_is_invalid() {
        let mut data = LspData {
            header: LspDataHeader {
                content_length: 123456,
                content_type: Some(String::from("some content type")),
            },
            content: LspDataContent(make_obj!({
                "method": "initialize",
                "params": {
                    "initializationOptions": {
                        "distant": {
                            "host": "some.host",
                            "port": "abcd",
                            "auth_key": "abc123"
                        }
                    }
                }
            })),
        };

        let err = data.take_session().unwrap_err();
        assert!(
            matches!(err, LspSessionError::InvalidSessionParams),
            "{:?}",
            err
        );
    }

    #[test]
    fn data_take_session_should_fail_if_missing_auth_key_param() {
        let mut data = LspData {
            header: LspDataHeader {
                content_length: 123456,
                content_type: Some(String::from("some content type")),
            },
            content: LspDataContent(make_obj!({
                "method": "initialize",
                "params": {
                    "initializationOptions": {
                        "distant": {
                            "host": "some.host",
                            "port": 22,
                        }
                    }
                }
            })),
        };

        let err = data.take_session().unwrap_err();
        assert!(
            matches!(err, LspSessionError::MissingSessionParams),
            "{:?}",
            err
        );
    }

    #[test]
    fn data_take_session_should_fail_if_auth_key_param_is_invalid() {
        let mut data = LspData {
            header: LspDataHeader {
                content_length: 123456,
                content_type: Some(String::from("some content type")),
            },
            content: LspDataContent(make_obj!({
                "method": "initialize",
                "params": {
                    "initializationOptions": {
                        "distant": {
                            "host": "some.host",
                            "port": 22,
                            "auth_key": 1234,
                        }
                    }
                }
            })),
        };

        let err = data.take_session().unwrap_err();
        assert!(
            matches!(err, LspSessionError::InvalidSessionParams),
            "{:?}",
            err
        );
    }

    #[test]
    fn data_take_session_should_fail_if_missing_method_field() {
        let mut data = LspData {
            header: LspDataHeader {
                content_length: 123456,
                content_type: Some(String::from("some content type")),
            },
            content: LspDataContent(make_obj!({
                "params": {
                    "initializationOptions": {
                        "distant": {
                            "host": "some.host",
                            "port": 22,
                            "auth_key": "abc123",
                        }
                    }
                }
            })),
        };

        let err = data.take_session().unwrap_err();
        assert!(
            matches!(err, LspSessionError::NotInitializeRequest),
            "{:?}",
            err
        );
    }

    #[test]
    fn data_take_session_should_fail_if_method_field_is_not_initialize() {
        let mut data = LspData {
            header: LspDataHeader {
                content_length: 123456,
                content_type: Some(String::from("some content type")),
            },
            content: LspDataContent(make_obj!({
                "method": "not initialize",
                "params": {
                    "initializationOptions": {
                        "distant": {
                            "host": "some.host",
                            "port": 22,
                            "auth_key": "abc123",
                        }
                    }
                }
            })),
        };

        let err = data.take_session().unwrap_err();
        assert!(
            matches!(err, LspSessionError::NotInitializeRequest),
            "{:?}",
            err
        );
    }

    #[test]
    fn header_parse_should_fail_if_missing_content_length() {
        let err = "Content-Type: some type\r\n\r\n"
            .parse::<LspDataHeader>()
            .unwrap_err();
        assert!(
            matches!(err, LspDataHeaderParseError::MissingContentLength),
            "{:?}",
            err
        );
    }

    #[test]
    fn header_parse_should_fail_if_content_length_invalid() {
        let err = "Content-Length: -1\r\n\r\n"
            .parse::<LspDataHeader>()
            .unwrap_err();
        assert!(
            matches!(err, LspDataHeaderParseError::InvalidContentLength(_)),
            "{:?}",
            err
        );
    }

    #[test]
    fn header_parse_should_fail_if_receive_an_unexpected_header_field() {
        let err = "Content-Length: 123\r\nUnknown-Field: abc\r\n\r\n"
            .parse::<LspDataHeader>()
            .unwrap_err();
        assert!(
            matches!(err, LspDataHeaderParseError::BadHeaderField),
            "{:?}",
            err
        );
    }

    #[test]
    fn header_parse_should_succeed_if_given_valid_content_length() {
        let header = "Content-Length: 123\r\n\r\n"
            .parse::<LspDataHeader>()
            .unwrap();
        assert_eq!(header.content_length, 123);
        assert_eq!(header.content_type, None);
    }

    #[test]
    fn header_parse_should_support_optional_content_type() {
        // Regular type
        let header = "Content-Length: 123\r\nContent-Type: some content type\r\n\r\n"
            .parse::<LspDataHeader>()
            .unwrap();
        assert_eq!(header.content_length, 123);
        assert_eq!(header.content_type.as_deref(), Some("some content type"));

        // Type with colons
        let header = "Content-Length: 123\r\nContent-Type: some:content:type\r\n\r\n"
            .parse::<LspDataHeader>()
            .unwrap();
        assert_eq!(header.content_length, 123);
        assert_eq!(header.content_type.as_deref(), Some("some:content:type"));
    }

    #[test]
    fn header_display_should_output_header_fields_with_appropriate_line_terminations() {
        // Without content type
        let header = LspDataHeader {
            content_length: 123,
            content_type: None,
        };
        assert_eq!(header.to_string(), "Content-Length: 123\r\n\r\n");

        // With content type
        let header = LspDataHeader {
            content_length: 123,
            content_type: Some(String::from("some type")),
        };
        assert_eq!(
            header.to_string(),
            "Content-Length: 123\r\nContent-Type: some type\r\n\r\n"
        );
    }

    #[test]
    fn content_parse_should_succeed_if_valid_json() {
        let content = "{\"hello\": \"world\"}".parse::<LspDataContent>().unwrap();
        assert_eq!(content.as_ref(), &make_obj!({"hello": "world"}));
    }

    #[test]
    fn content_parse_should_fail_if_invalid_json() {
        assert!(
            "not json".parse::<LspDataContent>().is_err(),
            "Unexpectedly succeeded"
        );
    }

    #[test]
    fn content_display_should_output_content_as_json() {
        let content = LspDataContent(make_obj!({"hello": "world"}));
        assert_eq!(content.to_string(), "{\n  \"hello\": \"world\"\n}");
    }
}
