use std::fmt;
use std::io::{self, BufRead};
use std::ops::{Deref, DerefMut};
use std::str::FromStr;
use std::string::FromUtf8Error;

use derive_more::{Display, Error, From};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Represents some data being communicated to/from an LSP consisting of a header and content part
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspMsg {
    /// Header-portion of some data related to LSP
    header: LspHeader,

    /// Content-portion of some data related to LSP
    content: LspContent,
}

#[derive(Debug, Display, Error, From)]
pub enum LspMsgParseError {
    /// When the received content is malformed
    BadContent(LspContentParseError),

    /// When the received header is malformed
    BadHeader(LspHeaderParseError),

    /// When a header line is not terminated in \r\n
    BadHeaderTermination,

    /// When input fails to be in UTF-8 format
    BadInput(FromUtf8Error),

    /// When some unexpected I/O error encountered
    IoError(io::Error),

    /// When EOF received before data fully acquired
    UnexpectedEof,
}

impl From<LspMsgParseError> for io::Error {
    fn from(x: LspMsgParseError) -> Self {
        match x {
            LspMsgParseError::BadContent(x) => x.into(),
            LspMsgParseError::BadHeader(x) => x.into(),
            LspMsgParseError::BadHeaderTermination => io::Error::new(
                io::ErrorKind::InvalidData,
                r"Received header line not terminated in \r\n",
            ),
            LspMsgParseError::BadInput(x) => io::Error::new(io::ErrorKind::InvalidData, x),
            LspMsgParseError::IoError(x) => x,
            LspMsgParseError::UnexpectedEof => io::Error::from(io::ErrorKind::UnexpectedEof),
        }
    }
}

impl LspMsg {
    /// Returns a reference to the header part
    pub fn header(&self) -> &LspHeader {
        &self.header
    }

    /// Returns a mutable reference to the header part
    pub fn mut_header(&mut self) -> &mut LspHeader {
        &mut self.header
    }

    /// Returns a reference to the content part
    pub fn content(&self) -> &LspContent {
        &self.content
    }

    /// Returns a mutable reference to the content part
    pub fn mut_content(&mut self) -> &mut LspContent {
        &mut self.content
    }

    /// Updates the header content length based on the current content
    pub fn refresh_content_length(&mut self) {
        self.header.content_length = self.content.to_string().len();
    }

    /// Attempts to read incoming lsp data from a buffered reader.
    ///
    /// Note that this is **blocking** while it waits on the header information (or EOF)!
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
    pub fn from_buf_reader<R: BufRead>(r: &mut R) -> Result<Self, LspMsgParseError> {
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
                return Err(LspMsgParseError::UnexpectedEof);
            }

            let line = &buf[start..end];

            // Check if we've gotten bad data
            if !line.ends_with("\r\n") {
                return Err(LspMsgParseError::BadHeaderTermination);

            // Check if we've received the header termination
            } else if line == "\r\n" {
                break;
            }
        }

        // Parse the header content so we know how much more to read
        let header = buf.parse::<LspHeader>()?;

        // Read remaining content
        let content = {
            let mut buf = vec![0u8; header.content_length];
            r.read_exact(&mut buf).map_err(|x| {
                if x.kind() == io::ErrorKind::UnexpectedEof {
                    LspMsgParseError::UnexpectedEof
                } else {
                    LspMsgParseError::IoError(x)
                }
            })?;
            String::from_utf8(buf)?.parse::<LspContent>()?
        };

        Ok(Self { header, content })
    }

    /// Converts into a vec of bytes representing the string format
    pub fn to_bytes(&self) -> Vec<u8> {
        self.to_string().into_bytes()
    }
}

impl fmt::Display for LspMsg {
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

impl FromStr for LspMsg {
    type Err = LspMsgParseError;

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
pub struct LspHeader {
    /// Length of content part in bytes
    pub content_length: usize,

    /// Mime type of content part, defaulting to
    /// application/vscode-jsonrpc; charset=utf-8
    pub content_type: Option<String>,
}

impl fmt::Display for LspHeader {
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
            write!(f, "Content-Type: {ty}\r\n")?;
        }

        write!(f, "\r\n")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Display, Error, From)]
pub enum LspHeaderParseError {
    MissingContentLength,
    InvalidContentLength(std::num::ParseIntError),
    BadHeaderField,
}

impl From<LspHeaderParseError> for io::Error {
    fn from(x: LspHeaderParseError) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, x)
    }
}

impl FromStr for LspHeader {
    type Err = LspHeaderParseError;

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
            match line.find(':') {
                Some(idx) if idx + 1 < line.len() => {
                    let name = &line[..idx];
                    let value = &line[(idx + 1)..];
                    match name {
                        "Content-Length" => content_length = Some(value.trim().parse()?),
                        "Content-Type" => content_type = Some(value.trim().to_string()),
                        _ => return Err(LspHeaderParseError::BadHeaderField),
                    }
                }
                _ => {
                    return Err(LspHeaderParseError::BadHeaderField);
                }
            }
        }

        match content_length {
            Some(content_length) => Ok(Self {
                content_length,
                content_type,
            }),
            None => Err(LspHeaderParseError::MissingContentLength),
        }
    }
}

/// Represents the content for LSP data
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LspContent(Map<String, Value>);

fn for_each_mut_string<F1, F2>(value: &mut Value, check: &F1, mutate: &mut F2)
where
    F1: Fn(&String) -> bool,
    F2: FnMut(&mut String),
{
    match value {
        Value::Object(obj) => {
            // Mutate values
            obj.values_mut()
                .for_each(|v| for_each_mut_string(v, check, mutate));

            // Mutate keys if necessary
            let keys: Vec<String> = obj
                .keys()
                .filter(|k| check(k))
                .map(ToString::to_string)
                .collect();
            for key in keys {
                if let Some((mut key, value)) = obj.remove_entry(&key) {
                    mutate(&mut key);
                    obj.insert(key, value);
                }
            }
        }
        Value::Array(items) => items
            .iter_mut()
            .for_each(|v| for_each_mut_string(v, check, mutate)),
        Value::String(s) => mutate(s),
        _ => {}
    }
}

fn swap_prefix(obj: &mut Map<String, Value>, old: &str, new: &str) {
    let check = |s: &String| s.starts_with(old);
    let mut mutate = |s: &mut String| {
        if let Some(pos) = s.find(old) {
            s.replace_range(pos..pos + old.len(), new);
        }
    };

    // Mutate values
    obj.values_mut()
        .for_each(|v| for_each_mut_string(v, &check, &mut mutate));

    // Mutate keys if necessary
    let keys: Vec<String> = obj
        .keys()
        .filter(|k| check(k))
        .map(ToString::to_string)
        .collect();
    for key in keys {
        if let Some((mut key, value)) = obj.remove_entry(&key) {
            mutate(&mut key);
            obj.insert(key, value);
        }
    }
}

impl LspContent {
    /// Converts all URIs with `file` as the scheme to `distant` instead
    pub fn convert_local_scheme_to_distant(&mut self) {
        self.convert_local_scheme_to("distant")
    }

    /// Converts all URIs with `file` as the scheme to `scheme` instead
    pub fn convert_local_scheme_to(&mut self, scheme: &str) {
        swap_prefix(&mut self.0, "file:", &format!("{scheme}:"));
    }

    /// Converts all URIs with `distant` as the scheme to `file` instead
    pub fn convert_distant_scheme_to_local(&mut self) {
        self.convert_scheme_to_local("distant")
    }

    /// Converts all URIs with `scheme` as the scheme to `file` instead
    pub fn convert_scheme_to_local(&mut self, scheme: &str) {
        swap_prefix(&mut self.0, &format!("{scheme}:"), "file:");
    }
}

impl AsRef<Map<String, Value>> for LspContent {
    fn as_ref(&self) -> &Map<String, Value> {
        &self.0
    }
}

impl Deref for LspContent {
    type Target = Map<String, Value>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for LspContent {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl fmt::Display for LspContent {
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
pub struct LspContentParseError(serde_json::Error);

impl From<LspContentParseError> for io::Error {
    fn from(x: LspContentParseError) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, x)
    }
}

impl FromStr for LspContent {
    type Err = LspContentParseError;

    /// Parses content in JSON form
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        serde_json::from_str(s).map_err(From::from)
    }
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    macro_rules! make_obj {
        ($($tail:tt)*) => {
            match serde_json::json!($($tail)*) {
                serde_json::Value::Object(x) => x,
                x => panic!("Got non-object: {:?}", x),
            }
        };
    }

    #[test]
    fn msg_display_should_output_header_and_content() {
        let msg = LspMsg {
            header: LspHeader {
                content_length: 123,
                content_type: Some(String::from("some content type")),
            },
            content: LspContent(make_obj!({"hello": "world"})),
        };

        let output = msg.to_string();
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
    fn msg_from_buf_reader_should_be_successful_if_valid_msg_received() {
        let mut input = io::Cursor::new(concat!(
            "Content-Length: 22\r\n",
            "Content-Type: some content type\r\n",
            "\r\n",
            "{\n",
            "  \"hello\": \"world\"\n",
            "}",
        ));
        let msg = LspMsg::from_buf_reader(&mut input).unwrap();
        assert_eq!(msg.header.content_length, 22);
        assert_eq!(
            msg.header.content_type.as_deref(),
            Some("some content type")
        );
        assert_eq!(msg.content.as_ref(), &make_obj!({ "hello": "world" }));
    }

    #[test]
    fn msg_from_buf_reader_should_fail_if_reach_eof_before_received_full_msg() {
        // No line termination
        let err = LspMsg::from_buf_reader(&mut io::Cursor::new("Content-Length: 22")).unwrap_err();
        assert!(
            matches!(err, LspMsgParseError::BadHeaderTermination),
            "{:?}",
            err
        );

        // Header doesn't finish
        let err = LspMsg::from_buf_reader(&mut io::Cursor::new(concat!(
            "Content-Length: 22\r\n",
            "Content-Type: some content type\r\n",
        )))
        .unwrap_err();
        assert!(matches!(err, LspMsgParseError::UnexpectedEof), "{:?}", err);

        // No content after header
        let err = LspMsg::from_buf_reader(&mut io::Cursor::new(concat!(
            "Content-Length: 22\r\n",
            "\r\n",
        )))
        .unwrap_err();
        assert!(matches!(err, LspMsgParseError::UnexpectedEof), "{:?}", err);
    }

    #[test]
    fn msg_from_buf_reader_should_fail_if_missing_proper_line_termination_for_header_field() {
        let err = LspMsg::from_buf_reader(&mut io::Cursor::new(concat!(
            "Content-Length: 22\n",
            "\r\n",
            "{\n",
            "  \"hello\": \"world\"\n",
            "}",
        )))
        .unwrap_err();
        assert!(
            matches!(err, LspMsgParseError::BadHeaderTermination),
            "{:?}",
            err
        );
    }

    #[test]
    fn msg_from_buf_reader_should_fail_if_bad_header_provided() {
        // Invalid content length
        let err = LspMsg::from_buf_reader(&mut io::Cursor::new(concat!(
            "Content-Length: -1\r\n",
            "\r\n",
            "{\n",
            "  \"hello\": \"world\"\n",
            "}",
        )))
        .unwrap_err();
        assert!(matches!(err, LspMsgParseError::BadHeader(_)), "{:?}", err);

        // Missing content length
        let err = LspMsg::from_buf_reader(&mut io::Cursor::new(concat!(
            "Content-Type: some content type\r\n",
            "\r\n",
            "{\n",
            "  \"hello\": \"world\"\n",
            "}",
        )))
        .unwrap_err();
        assert!(matches!(err, LspMsgParseError::BadHeader(_)), "{:?}", err);
    }

    #[test]
    fn msg_from_buf_reader_should_fail_if_bad_content_provided() {
        // Not full content
        let err = LspMsg::from_buf_reader(&mut io::Cursor::new(concat!(
            "Content-Length: 21\r\n",
            "\r\n",
            "{\n",
            "  \"hello\": \"world\"\n",
        )))
        .unwrap_err();
        assert!(matches!(err, LspMsgParseError::BadContent(_)), "{:?}", err);
    }

    #[test]
    fn msg_from_buf_reader_should_fail_if_non_utf8_msg_encountered_for_content() {
        // Not utf-8 content
        let mut raw = b"Content-Length: 2\r\n\r\n".to_vec();
        raw.extend(vec![0, 159]);

        let err = LspMsg::from_buf_reader(&mut io::Cursor::new(raw)).unwrap_err();
        assert!(matches!(err, LspMsgParseError::BadInput(_)), "{:?}", err);
    }

    #[test]
    fn header_parse_should_fail_if_missing_content_length() {
        let err = "Content-Type: some type\r\n\r\n"
            .parse::<LspHeader>()
            .unwrap_err();
        assert!(
            matches!(err, LspHeaderParseError::MissingContentLength),
            "{:?}",
            err
        );
    }

    #[test]
    fn header_parse_should_fail_if_content_length_invalid() {
        let err = "Content-Length: -1\r\n\r\n"
            .parse::<LspHeader>()
            .unwrap_err();
        assert!(
            matches!(err, LspHeaderParseError::InvalidContentLength(_)),
            "{:?}",
            err
        );
    }

    #[test]
    fn header_parse_should_fail_if_receive_an_unexpected_header_field() {
        let err = "Content-Length: 123\r\nUnknown-Field: abc\r\n\r\n"
            .parse::<LspHeader>()
            .unwrap_err();
        assert!(
            matches!(err, LspHeaderParseError::BadHeaderField),
            "{:?}",
            err
        );
    }

    #[test]
    fn header_parse_should_succeed_if_given_valid_content_length() {
        let header = "Content-Length: 123\r\n\r\n".parse::<LspHeader>().unwrap();
        assert_eq!(header.content_length, 123);
        assert_eq!(header.content_type, None);
    }

    #[test]
    fn header_parse_should_support_optional_content_type() {
        // Regular type
        let header = "Content-Length: 123\r\nContent-Type: some content type\r\n\r\n"
            .parse::<LspHeader>()
            .unwrap();
        assert_eq!(header.content_length, 123);
        assert_eq!(header.content_type.as_deref(), Some("some content type"));

        // Type with colons
        let header = "Content-Length: 123\r\nContent-Type: some:content:type\r\n\r\n"
            .parse::<LspHeader>()
            .unwrap();
        assert_eq!(header.content_length, 123);
        assert_eq!(header.content_type.as_deref(), Some("some:content:type"));
    }

    #[test]
    fn header_display_should_output_header_fields_with_appropriate_line_terminations() {
        // Without content type
        let header = LspHeader {
            content_length: 123,
            content_type: None,
        };
        assert_eq!(header.to_string(), "Content-Length: 123\r\n\r\n");

        // With content type
        let header = LspHeader {
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
        let content = "{\"hello\": \"world\"}".parse::<LspContent>().unwrap();
        assert_eq!(content.as_ref(), &make_obj!({"hello": "world"}));
    }

    #[test]
    fn content_parse_should_fail_if_invalid_json() {
        assert!(
            "not json".parse::<LspContent>().is_err(),
            "Unexpectedly succeeded"
        );
    }

    #[test]
    fn content_display_should_output_content_as_json() {
        let content = LspContent(make_obj!({"hello": "world"}));
        assert_eq!(content.to_string(), "{\n  \"hello\": \"world\"\n}");
    }

    #[test]
    fn content_convert_local_scheme_to_distant_should_convert_keys_and_values() {
        let mut content = LspContent(make_obj!({
            "distant://key1": "file://value1",
            "file://key2": "distant://value2",
            "key3": ["file://value3", "distant://value4"],
            "key4": {
                "distant://key5": "file://value5",
                "file://key6": "distant://value6",
                "key7": [
                    {
                        "distant://key8": "file://value8",
                        "file://key9": "distant://value9",
                    }
                ]
            },
            "key10": null,
            "key11": 123,
            "key12": true,
        }));

        content.convert_local_scheme_to_distant();
        assert_eq!(
            content.0,
            make_obj!({
                "distant://key1": "distant://value1",
                "distant://key2": "distant://value2",
                "key3": ["distant://value3", "distant://value4"],
                "key4": {
                    "distant://key5": "distant://value5",
                    "distant://key6": "distant://value6",
                    "key7": [
                        {
                            "distant://key8": "distant://value8",
                            "distant://key9": "distant://value9",
                        }
                    ]
                },
                "key10": null,
                "key11": 123,
                "key12": true,
            })
        );
    }

    #[test]
    fn content_convert_local_scheme_to_should_convert_keys_and_values() {
        let mut content = LspContent(make_obj!({
            "distant://key1": "file://value1",
            "file://key2": "distant://value2",
            "key3": ["file://value3", "distant://value4"],
            "key4": {
                "distant://key5": "file://value5",
                "file://key6": "distant://value6",
                "key7": [
                    {
                        "distant://key8": "file://value8",
                        "file://key9": "distant://value9",
                    }
                ]
            },
            "key10": null,
            "key11": 123,
            "key12": true,
        }));

        content.convert_local_scheme_to("custom");
        assert_eq!(
            content.0,
            make_obj!({
                "distant://key1": "custom://value1",
                "custom://key2": "distant://value2",
                "key3": ["custom://value3", "distant://value4"],
                "key4": {
                    "distant://key5": "custom://value5",
                    "custom://key6": "distant://value6",
                    "key7": [
                        {
                            "distant://key8": "custom://value8",
                            "custom://key9": "distant://value9",
                        }
                    ]
                },
                "key10": null,
                "key11": 123,
                "key12": true,
            })
        );
    }

    #[test]
    fn content_convert_distant_scheme_to_local_should_convert_keys_and_values() {
        let mut content = LspContent(make_obj!({
            "distant://key1": "file://value1",
            "file://key2": "distant://value2",
            "key3": ["file://value3", "distant://value4"],
            "key4": {
                "distant://key5": "file://value5",
                "file://key6": "distant://value6",
                "key7": [
                    {
                        "distant://key8": "file://value8",
                        "file://key9": "distant://value9",
                    }
                ]
            },
            "key10": null,
            "key11": 123,
            "key12": true,
        }));

        content.convert_distant_scheme_to_local();
        assert_eq!(
            content.0,
            make_obj!({
                "file://key1": "file://value1",
                "file://key2": "file://value2",
                "key3": ["file://value3", "file://value4"],
                "key4": {
                    "file://key5": "file://value5",
                    "file://key6": "file://value6",
                    "key7": [
                        {
                            "file://key8": "file://value8",
                            "file://key9": "file://value9",
                        }
                    ]
                },
                "key10": null,
                "key11": 123,
                "key12": true,
            })
        );
    }

    #[test]
    fn content_convert_scheme_to_local_should_convert_keys_and_values() {
        let mut content = LspContent(make_obj!({
            "custom://key1": "file://value1",
            "file://key2": "custom://value2",
            "key3": ["file://value3", "custom://value4"],
            "key4": {
                "custom://key5": "file://value5",
                "file://key6": "custom://value6",
                "key7": [
                    {
                        "custom://key8": "file://value8",
                        "file://key9": "custom://value9",
                    }
                ]
            },
            "key10": null,
            "key11": 123,
            "key12": true,
        }));

        content.convert_scheme_to_local("custom");
        assert_eq!(
            content.0,
            make_obj!({
                "file://key1": "file://value1",
                "file://key2": "file://value2",
                "key3": ["file://value3", "file://value4"],
                "key4": {
                    "file://key5": "file://value5",
                    "file://key6": "file://value6",
                    "key7": [
                        {
                            "file://key8": "file://value8",
                            "file://key9": "file://value9",
                        }
                    ]
                },
                "key10": null,
                "key11": 123,
                "key12": true,
            })
        );
    }

    #[test]
    fn msg_from_str_should_parse_valid_lsp_message() {
        let input = concat!(
            "Content-Length: 22\r\n",
            "Content-Type: some content type\r\n",
            "\r\n",
            "{\n",
            "  \"hello\": \"world\"\n",
            "}",
        );
        let msg: LspMsg = input.parse().unwrap();
        assert_eq!(msg.header().content_length, 22);
        assert_eq!(
            msg.header().content_type.as_deref(),
            Some("some content type")
        );
        assert_eq!(msg.content().as_ref(), &make_obj!({"hello": "world"}));
    }

    #[test]
    fn msg_from_str_should_fail_on_empty_string() {
        let err = "".parse::<LspMsg>().unwrap_err();
        assert!(matches!(err, LspMsgParseError::UnexpectedEof), "{:?}", err);
    }

    #[test]
    fn msg_to_bytes_should_produce_valid_utf8_bytes() {
        let msg = LspMsg {
            header: LspHeader {
                content_length: 22,
                content_type: None,
            },
            content: LspContent(make_obj!({"hello": "world"})),
        };
        let bytes = msg.to_bytes();
        let as_string = String::from_utf8(bytes).unwrap();
        assert!(as_string.starts_with("Content-Length: 22\r\n"));
        assert!(as_string.contains("\"hello\""));
    }

    #[test]
    fn msg_to_bytes_should_round_trip_through_from_buf_reader() {
        let original = LspMsg {
            header: LspHeader {
                content_length: 22,
                content_type: None,
            },
            content: LspContent(make_obj!({"hello": "world"})),
        };
        let bytes = original.to_bytes();
        let parsed = LspMsg::from_buf_reader(&mut io::Cursor::new(bytes)).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn msg_refresh_content_length_should_update_header_to_match_content() {
        let mut msg = LspMsg {
            header: LspHeader {
                content_length: 0,
                content_type: None,
            },
            content: LspContent(make_obj!({"hello": "world"})),
        };
        assert_eq!(msg.header().content_length, 0);

        msg.refresh_content_length();

        let expected_len = msg.content().to_string().len();
        assert_eq!(msg.header().content_length, expected_len);
        assert_ne!(msg.header().content_length, 0);
    }

    #[test]
    fn msg_header_and_content_accessors_should_work() {
        let mut msg = LspMsg {
            header: LspHeader {
                content_length: 10,
                content_type: Some("text/plain".to_string()),
            },
            content: LspContent(make_obj!({"key": "val"})),
        };

        // Immutable accessors
        assert_eq!(msg.header().content_length, 10);
        assert_eq!(msg.content().get("key").unwrap(), "val");

        // Mutable accessors
        msg.mut_header().content_length = 20;
        assert_eq!(msg.header().content_length, 20);

        msg.mut_content()
            .insert("new_key".to_string(), Value::Bool(true));
        assert_eq!(msg.content().get("new_key").unwrap(), true);
    }

    #[test]
    fn msg_from_buf_reader_should_read_multiple_messages_consecutively() {
        let msg1 = concat!(
            "Content-Length: 22\r\n",
            "\r\n",
            "{\n",
            "  \"hello\": \"world\"\n",
            "}",
        );
        let msg2 = concat!(
            "Content-Length: 14\r\n",
            "\r\n",
            "{\n",
            "  \"a\": \"b\"\n",
            "}",
        );
        let combined = format!("{msg1}{msg2}");
        let mut cursor = io::Cursor::new(combined);

        let parsed1 = LspMsg::from_buf_reader(&mut cursor).unwrap();
        assert_eq!(parsed1.content().as_ref(), &make_obj!({"hello": "world"}));

        let parsed2 = LspMsg::from_buf_reader(&mut cursor).unwrap();
        assert_eq!(parsed2.content().as_ref(), &make_obj!({"a": "b"}));
    }

    #[test]
    fn msg_from_buf_reader_should_succeed_with_content_length_zero_and_empty_json_object() {
        let input = "Content-Length: 2\r\n\r\n{}";
        let msg = LspMsg::from_buf_reader(&mut io::Cursor::new(input)).unwrap();
        assert_eq!(msg.header().content_length, 2);
        assert!(msg.content().is_empty());
    }

    #[test]
    fn msg_from_buf_reader_should_fail_if_content_shorter_than_content_length() {
        // content-length says 100 but only 2 bytes of content
        let input = "Content-Length: 100\r\n\r\n{}";
        let err = LspMsg::from_buf_reader(&mut io::Cursor::new(input)).unwrap_err();
        assert!(matches!(err, LspMsgParseError::UnexpectedEof), "{:?}", err);
    }

    #[test]
    fn header_parse_should_fail_if_line_has_no_colon() {
        let err = "Content-Length 123\r\n\r\n"
            .parse::<LspHeader>()
            .unwrap_err();
        assert!(
            matches!(err, LspHeaderParseError::BadHeaderField),
            "{:?}",
            err
        );
    }

    #[test]
    fn header_parse_should_fail_if_colon_is_last_char() {
        // "X:" has colon at index 1, but idx+1 == line.len() so the condition fails
        let err = "X:\r\n\r\n".parse::<LspHeader>().unwrap_err();
        assert!(
            matches!(err, LspHeaderParseError::BadHeaderField),
            "{:?}",
            err
        );
    }

    #[test]
    fn header_parse_should_succeed_with_only_content_length_no_trailing_crlf() {
        // FromStr should handle the case where there is no trailing \r\n\r\n
        // because split("\r\n") + filter(empty) still yields the header line
        let header = "Content-Length: 42".parse::<LspHeader>().unwrap();
        assert_eq!(header.content_length, 42);
        assert_eq!(header.content_type, None);
    }

    #[test]
    fn lsp_msg_parse_error_should_convert_to_io_error_for_all_variants() {
        // BadContent
        let bad_content_err: LspMsgParseError = LspMsgParseError::BadContent(
            serde_json::from_str::<LspContent>("!!!")
                .unwrap_err()
                .into(),
        );
        let io_err: io::Error = bad_content_err.into();
        assert_eq!(io_err.kind(), io::ErrorKind::InvalidData);

        // BadHeader
        let bad_header_err: LspMsgParseError =
            LspMsgParseError::BadHeader(LspHeaderParseError::MissingContentLength);
        let io_err: io::Error = bad_header_err.into();
        assert_eq!(io_err.kind(), io::ErrorKind::InvalidData);

        // BadHeaderTermination
        let io_err: io::Error = LspMsgParseError::BadHeaderTermination.into();
        assert_eq!(io_err.kind(), io::ErrorKind::InvalidData);

        // BadInput
        let bad_utf8 = String::from_utf8(vec![0, 159, 146, 150]).unwrap_err();
        let io_err: io::Error = LspMsgParseError::BadInput(bad_utf8).into();
        assert_eq!(io_err.kind(), io::ErrorKind::InvalidData);

        // IoError
        let io_err: io::Error = LspMsgParseError::IoError(io::Error::other("test")).into();
        assert_eq!(io_err.kind(), io::ErrorKind::Other);

        // UnexpectedEof
        let io_err: io::Error = LspMsgParseError::UnexpectedEof.into();
        assert_eq!(io_err.kind(), io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn lsp_header_parse_error_should_convert_to_io_error() {
        let io_err: io::Error = LspHeaderParseError::MissingContentLength.into();
        assert_eq!(io_err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn lsp_content_parse_error_should_convert_to_io_error() {
        let content_err = "not json".parse::<LspContent>().unwrap_err();
        let io_err: io::Error = content_err.into();
        assert_eq!(io_err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn content_deref_should_expose_map_methods() {
        let content = LspContent(make_obj!({"a": 1, "b": 2}));
        // Deref to Map<String, Value>
        assert_eq!(content.len(), 2);
        assert!(content.contains_key("a"));
    }

    #[test]
    fn content_deref_mut_should_allow_modification() {
        let mut content = LspContent(make_obj!({"a": 1}));
        content.insert("b".to_string(), Value::from(2));
        assert_eq!(content.len(), 2);
    }

    #[test]
    fn content_as_ref_should_return_inner_map() {
        let content = LspContent(make_obj!({"x": "y"}));
        let map: &Map<String, Value> = content.as_ref();
        assert_eq!(map.get("x").unwrap(), "y");
    }

    #[test]
    fn header_display_round_trip_through_parse() {
        let header = LspHeader {
            content_length: 456,
            content_type: Some("application/json".to_string()),
        };
        let displayed = header.to_string();
        let parsed: LspHeader = displayed.parse().unwrap();
        assert_eq!(parsed.content_length, 456);
        assert_eq!(parsed.content_type.as_deref(), Some("application/json"));
    }

    #[test]
    fn msg_display_round_trip_through_from_str() {
        let msg = LspMsg {
            header: LspHeader {
                content_length: 22,
                content_type: None,
            },
            content: LspContent(make_obj!({"hello": "world"})),
        };
        let displayed = msg.to_string();
        let parsed: LspMsg = displayed.parse().unwrap();
        assert_eq!(parsed, msg);
    }

    #[test]
    fn msg_from_buf_reader_should_handle_content_type_with_charset() {
        let input = concat!(
            "Content-Length: 22\r\n",
            "Content-Type: application/vscode-jsonrpc; charset=utf-8\r\n",
            "\r\n",
            "{\n",
            "  \"hello\": \"world\"\n",
            "}",
        );
        let msg = LspMsg::from_buf_reader(&mut io::Cursor::new(input)).unwrap();
        assert_eq!(
            msg.header().content_type.as_deref(),
            Some("application/vscode-jsonrpc; charset=utf-8")
        );
    }
}
