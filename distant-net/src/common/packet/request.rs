use std::borrow::Cow;
use std::{io, str};

use derive_more::{Display, Error};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::{read_header_bytes, read_key_eq, read_str_bytes, Header, Id};
use crate::common::utils;
use crate::header;

/// Represents a request to send
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request<T> {
    /// Optional header data to include with request
    #[serde(default, skip_serializing_if = "Header::is_empty")]
    pub header: Header,

    /// Unique id associated with the request
    pub id: Id,

    /// Payload associated with the request
    pub payload: T,
}

impl<T> Request<T> {
    /// Creates a new request with a random, unique id and no header data
    pub fn new(payload: T) -> Self {
        Self {
            header: header!(),
            id: rand::random::<u64>().to_string(),
            payload,
        }
    }
}

impl<T> Request<T>
where
    T: Serialize,
{
    /// Serializes the request into bytes
    pub fn to_vec(&self) -> io::Result<Vec<u8>> {
        utils::serialize_to_vec(self)
    }

    /// Serializes the request's payload into bytes
    pub fn to_payload_vec(&self) -> io::Result<Vec<u8>> {
        utils::serialize_to_vec(&self.payload)
    }

    /// Attempts to convert a typed request to an untyped request
    pub fn to_untyped_request(&self) -> io::Result<UntypedRequest> {
        Ok(UntypedRequest {
            header: Cow::Owned(if !self.header.is_empty() {
                utils::serialize_to_vec(&self.header)?
            } else {
                Vec::new()
            }),
            id: Cow::Borrowed(&self.id),
            payload: Cow::Owned(self.to_payload_vec()?),
        })
    }
}

impl<T> Request<T>
where
    T: DeserializeOwned,
{
    /// Deserializes the request from bytes
    pub fn from_slice(slice: &[u8]) -> io::Result<Self> {
        utils::deserialize_from_slice(slice)
    }
}

impl<T> From<T> for Request<T> {
    fn from(payload: T) -> Self {
        Self::new(payload)
    }
}

/// Error encountered when attempting to parse bytes as an untyped request
#[derive(Copy, Clone, Debug, Display, Error, PartialEq, Eq, Hash)]
pub enum UntypedRequestParseError {
    /// When the bytes do not represent a request
    WrongType,

    /// When a header should be present, but the key is wrong
    InvalidHeaderKey,

    /// When a header should be present, but the header bytes are wrong
    InvalidHeader,

    /// When the key for the id is wrong
    InvalidIdKey,

    /// When the id is not a valid UTF-8 string
    InvalidId,

    /// When the key for the payload is wrong
    InvalidPayloadKey,
}

#[inline]
fn header_is_empty(header: &[u8]) -> bool {
    header.is_empty()
}

/// Represents a request to send whose payload is bytes instead of a specific type
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UntypedRequest<'a> {
    /// Header data associated with the request as bytes
    #[serde(default, skip_serializing_if = "header_is_empty")]
    pub header: Cow<'a, [u8]>,

    /// Unique id associated with the request
    pub id: Cow<'a, str>,

    /// Payload associated with the request as bytes
    pub payload: Cow<'a, [u8]>,
}

impl<'a> UntypedRequest<'a> {
    /// Attempts to convert an untyped request to a typed request
    pub fn to_typed_request<T: DeserializeOwned>(&self) -> io::Result<Request<T>> {
        Ok(Request {
            header: if header_is_empty(&self.header) {
                header!()
            } else {
                utils::deserialize_from_slice(&self.header)?
            },
            id: self.id.to_string(),
            payload: utils::deserialize_from_slice(&self.payload)?,
        })
    }

    /// Convert into a borrowed version
    pub fn as_borrowed(&self) -> UntypedRequest<'_> {
        UntypedRequest {
            header: match &self.header {
                Cow::Borrowed(x) => Cow::Borrowed(x),
                Cow::Owned(x) => Cow::Borrowed(x.as_slice()),
            },
            id: match &self.id {
                Cow::Borrowed(x) => Cow::Borrowed(x),
                Cow::Owned(x) => Cow::Borrowed(x.as_str()),
            },
            payload: match &self.payload {
                Cow::Borrowed(x) => Cow::Borrowed(x),
                Cow::Owned(x) => Cow::Borrowed(x.as_slice()),
            },
        }
    }

    /// Convert into an owned version
    pub fn into_owned(self) -> UntypedRequest<'static> {
        UntypedRequest {
            header: match self.header {
                Cow::Borrowed(x) => Cow::Owned(x.to_vec()),
                Cow::Owned(x) => Cow::Owned(x),
            },
            id: match self.id {
                Cow::Borrowed(x) => Cow::Owned(x.to_string()),
                Cow::Owned(x) => Cow::Owned(x),
            },
            payload: match self.payload {
                Cow::Borrowed(x) => Cow::Owned(x.to_vec()),
                Cow::Owned(x) => Cow::Owned(x),
            },
        }
    }

    /// Updates the header of the request to the given `header`.
    pub fn set_header(&mut self, header: impl IntoIterator<Item = u8>) {
        self.header = Cow::Owned(header.into_iter().collect());
    }

    /// Updates the id of the request to the given `id`.
    pub fn set_id(&mut self, id: impl Into<String>) {
        self.id = Cow::Owned(id.into());
    }

    /// Allocates a new collection of bytes representing the request.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = vec![];

        let has_header = !header_is_empty(&self.header);
        if has_header {
            rmp::encode::write_map_len(&mut bytes, 3).unwrap();
        } else {
            rmp::encode::write_map_len(&mut bytes, 2).unwrap();
        }

        if has_header {
            rmp::encode::write_str(&mut bytes, "header").unwrap();
            bytes.extend_from_slice(&self.header);
        }

        rmp::encode::write_str(&mut bytes, "id").unwrap();
        rmp::encode::write_str(&mut bytes, &self.id).unwrap();

        rmp::encode::write_str(&mut bytes, "payload").unwrap();
        bytes.extend_from_slice(&self.payload);

        bytes
    }

    /// Parses a collection of bytes, returning a partial request if it can be potentially
    /// represented as a [`Request`] depending on the payload.
    ///
    /// NOTE: This supports parsing an invalid request where the payload would not properly
    /// deserialize, but the bytes themselves represent a complete request of some kind.
    pub fn from_slice(input: &'a [u8]) -> Result<Self, UntypedRequestParseError> {
        if input.is_empty() {
            return Err(UntypedRequestParseError::WrongType);
        }

        let has_header = match rmp::Marker::from_u8(input[0]) {
            rmp::Marker::FixMap(2) => false,
            rmp::Marker::FixMap(3) => true,
            _ => return Err(UntypedRequestParseError::WrongType),
        };

        // Advance position by marker
        let input = &input[1..];

        // Parse the header if we have one
        let (header, input) = if has_header {
            let (_, input) = read_key_eq(input, "header")
                .map_err(|_| UntypedRequestParseError::InvalidHeaderKey)?;

            let (header, input) =
                read_header_bytes(input).map_err(|_| UntypedRequestParseError::InvalidHeader)?;
            (header, input)
        } else {
            ([0u8; 0].as_slice(), input)
        };

        // Validate that next field is id
        let (_, input) =
            read_key_eq(input, "id").map_err(|_| UntypedRequestParseError::InvalidIdKey)?;

        // Get the id itself
        let (id, input) = read_str_bytes(input).map_err(|_| UntypedRequestParseError::InvalidId)?;

        // Validate that final field is payload
        let (_, input) = read_key_eq(input, "payload")
            .map_err(|_| UntypedRequestParseError::InvalidPayloadKey)?;

        let header = Cow::Borrowed(header);
        let id = Cow::Borrowed(id);
        let payload = Cow::Borrowed(input);

        Ok(Self {
            header,
            id,
            payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    const TRUE_BYTE: u8 = 0xc3;
    const NEVER_USED_BYTE: u8 = 0xc1;

    // fixstr of 6 bytes with str "header"
    const HEADER_FIELD_BYTES: &[u8] = &[0xa6, b'h', b'e', b'a', b'd', b'e', b'r'];

    // fixmap of 2 objects with
    // 1. key fixstr "key" and value fixstr "value"
    // 1. key fixstr "num" and value fixint 123
    const HEADER_BYTES: &[u8] = &[
        0x82, // valid map with 2 pair
        0xa3, b'k', b'e', b'y', // key: "key"
        0xa5, b'v', b'a', b'l', b'u', b'e', // value: "value"
        0xa3, b'n', b'u', b'm', // key: "num"
        0x7b, // value: 123
    ];

    // fixstr of 2 bytes with str "id"
    const ID_FIELD_BYTES: &[u8] = &[0xa2, b'i', b'd'];

    // fixstr of 7 bytes with str "payload"
    const PAYLOAD_FIELD_BYTES: &[u8] = &[0xa7, b'p', b'a', b'y', b'l', b'o', b'a', b'd'];

    // fixstr of 4 bytes with str "test"
    const TEST_STR_BYTES: &[u8] = &[0xa4, b't', b'e', b's', b't'];

    #[test]
    fn untyped_request_should_support_converting_to_bytes() {
        let bytes = Request {
            header: header!(),
            id: "some id".to_string(),
            payload: true,
        }
        .to_vec()
        .unwrap();

        let untyped_request = UntypedRequest::from_slice(&bytes).unwrap();
        assert_eq!(untyped_request.to_bytes(), bytes);
    }

    #[test]
    fn untyped_request_should_support_converting_to_bytes_with_header() {
        let bytes = Request {
            header: header!("key" -> 123),
            id: "some id".to_string(),
            payload: true,
        }
        .to_vec()
        .unwrap();

        let untyped_request = UntypedRequest::from_slice(&bytes).unwrap();
        assert_eq!(untyped_request.to_bytes(), bytes);
    }

    #[test]
    fn untyped_request_should_support_parsing_from_request_bytes_with_header() {
        let bytes = Request {
            header: header!("key" -> 123),
            id: "some id".to_string(),
            payload: true,
        }
        .to_vec()
        .unwrap();

        assert_eq!(
            UntypedRequest::from_slice(&bytes),
            Ok(UntypedRequest {
                header: Cow::Owned(utils::serialize_to_vec(&header!("key" -> 123)).unwrap()),
                id: Cow::Borrowed("some id"),
                payload: Cow::Owned(vec![TRUE_BYTE]),
            })
        );
    }

    #[test]
    fn untyped_request_should_support_parsing_from_request_bytes_with_valid_payload() {
        let bytes = Request {
            header: header!(),
            id: "some id".to_string(),
            payload: true,
        }
        .to_vec()
        .unwrap();

        assert_eq!(
            UntypedRequest::from_slice(&bytes),
            Ok(UntypedRequest {
                header: Cow::Owned(vec![]),
                id: Cow::Borrowed("some id"),
                payload: Cow::Owned(vec![TRUE_BYTE]),
            })
        );
    }

    #[test]
    fn untyped_request_should_support_parsing_from_request_bytes_with_invalid_payload() {
        // Request with id < 32 bytes
        let mut bytes = Request {
            header: header!(),
            id: "".to_string(),
            payload: true,
        }
        .to_vec()
        .unwrap();

        // Push never used byte in msgpack
        bytes.push(NEVER_USED_BYTE);

        // We don't actually check for a valid payload, so the extra byte shows up
        assert_eq!(
            UntypedRequest::from_slice(&bytes),
            Ok(UntypedRequest {
                header: Cow::Owned(vec![]),
                id: Cow::Owned("".to_string()),
                payload: Cow::Owned(vec![TRUE_BYTE, NEVER_USED_BYTE]),
            })
        );
    }

    #[test]
    fn untyped_request_should_support_parsing_full_request() {
        let input = [
            &[0x83],
            HEADER_FIELD_BYTES,
            HEADER_BYTES,
            ID_FIELD_BYTES,
            TEST_STR_BYTES,
            PAYLOAD_FIELD_BYTES,
            &[TRUE_BYTE],
        ]
        .concat();

        // Convert into typed so we can test
        let untyped_request = UntypedRequest::from_slice(&input).unwrap();
        let request: Request<bool> = untyped_request.to_typed_request().unwrap();

        assert_eq!(request.header, header!("key" -> "value", "num" -> 123));
        assert_eq!(request.id, "test");
        assert!(request.payload);
    }

    #[test]
    fn untyped_request_should_fail_to_parse_if_given_bytes_not_representing_a_request() {
        // Empty byte slice
        assert_eq!(
            UntypedRequest::from_slice(&[]),
            Err(UntypedRequestParseError::WrongType)
        );

        // Wrong starting byte
        assert_eq!(
            UntypedRequest::from_slice(&[0x00]),
            Err(UntypedRequestParseError::WrongType)
        );

        // Wrong starting byte (fixmap of 0 fields)
        assert_eq!(
            UntypedRequest::from_slice(&[0x80]),
            Err(UntypedRequestParseError::WrongType)
        );

        // Invalid header key
        assert_eq!(
            UntypedRequest::from_slice(
                [
                    &[0x83],
                    &[0xa0], // header key would be defined here, set to empty str
                    HEADER_BYTES,
                    ID_FIELD_BYTES,
                    TEST_STR_BYTES,
                    PAYLOAD_FIELD_BYTES,
                    &[TRUE_BYTE],
                ]
                .concat()
                .as_slice()
            ),
            Err(UntypedRequestParseError::InvalidHeaderKey)
        );

        // Invalid header bytes
        assert_eq!(
            UntypedRequest::from_slice(
                [
                    &[0x83],
                    HEADER_FIELD_BYTES,
                    &[0xa0], // header would be defined here, set to empty str
                    ID_FIELD_BYTES,
                    TEST_STR_BYTES,
                    PAYLOAD_FIELD_BYTES,
                    &[TRUE_BYTE],
                ]
                .concat()
                .as_slice()
            ),
            Err(UntypedRequestParseError::InvalidHeader)
        );

        // Missing fields (corrupt data)
        assert_eq!(
            UntypedRequest::from_slice(&[0x82]),
            Err(UntypedRequestParseError::InvalidIdKey)
        );

        // Missing id field (has valid data itself)
        assert_eq!(
            UntypedRequest::from_slice(
                [
                    &[0x82],
                    &[0xa0], // id would be defined here, set to empty str
                    TEST_STR_BYTES,
                    PAYLOAD_FIELD_BYTES,
                    &[TRUE_BYTE],
                ]
                .concat()
                .as_slice()
            ),
            Err(UntypedRequestParseError::InvalidIdKey)
        );

        // Non-str id field value
        assert_eq!(
            UntypedRequest::from_slice(
                [
                    &[0x82],
                    ID_FIELD_BYTES,
                    &[TRUE_BYTE], // id value set to boolean
                    PAYLOAD_FIELD_BYTES,
                    &[TRUE_BYTE],
                ]
                .concat()
                .as_slice()
            ),
            Err(UntypedRequestParseError::InvalidId)
        );

        // Non-utf8 id field value
        assert_eq!(
            UntypedRequest::from_slice(
                [
                    &[0x82],
                    ID_FIELD_BYTES,
                    &[0xa4, 0, 159, 146, 150],
                    PAYLOAD_FIELD_BYTES,
                    &[TRUE_BYTE],
                ]
                .concat()
                .as_slice()
            ),
            Err(UntypedRequestParseError::InvalidId)
        );

        // Missing payload field (has valid data itself)
        assert_eq!(
            UntypedRequest::from_slice(
                [
                    &[0x82],
                    ID_FIELD_BYTES,
                    TEST_STR_BYTES,
                    &[0xa0], // payload would be defined here, set to empty str
                    &[TRUE_BYTE],
                ]
                .concat()
                .as_slice()
            ),
            Err(UntypedRequestParseError::InvalidPayloadKey)
        );
    }
}
