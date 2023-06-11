use std::borrow::Cow;
use std::io;

use derive_more::{Display, Error};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::{read_header_bytes, read_key_eq, read_str_bytes, Header, Id};
use crate::common::utils;
use crate::header;

/// Represents a response received related to some response
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response<T> {
    /// Optional header data to include with response
    #[serde(default, skip_serializing_if = "Header::is_empty")]
    pub header: Header,

    /// Unique id associated with the response
    pub id: Id,

    /// Unique id associated with the response that triggered the response
    pub origin_id: Id,

    /// Payload associated with the response
    pub payload: T,
}

impl<T> Response<T> {
    /// Creates a new response with a random, unique id and no header data
    pub fn new(origin_id: Id, payload: T) -> Self {
        Self {
            header: header!(),
            id: rand::random::<u64>().to_string(),
            origin_id,
            payload,
        }
    }
}

impl<T> Response<T>
where
    T: Serialize,
{
    /// Serializes the response into bytes
    pub fn to_vec(&self) -> std::io::Result<Vec<u8>> {
        utils::serialize_to_vec(self)
    }

    /// Serializes the response's payload into bytes
    pub fn to_payload_vec(&self) -> io::Result<Vec<u8>> {
        utils::serialize_to_vec(&self.payload)
    }

    /// Attempts to convert a typed response to an untyped response
    pub fn to_untyped_response(&self) -> io::Result<UntypedResponse> {
        Ok(UntypedResponse {
            header: Cow::Owned(if !self.header.is_empty() {
                utils::serialize_to_vec(&self.header)?
            } else {
                Vec::new()
            }),
            id: Cow::Borrowed(&self.id),
            origin_id: Cow::Borrowed(&self.origin_id),
            payload: Cow::Owned(self.to_payload_vec()?),
        })
    }
}

impl<T> Response<T>
where
    T: DeserializeOwned,
{
    /// Deserializes the response from bytes
    pub fn from_slice(slice: &[u8]) -> std::io::Result<Self> {
        utils::deserialize_from_slice(slice)
    }
}

/// Error encountered when attempting to parse bytes as an untyped response
#[derive(Copy, Clone, Debug, Display, Error, PartialEq, Eq, Hash)]
pub enum UntypedResponseParseError {
    /// When the bytes do not represent a response
    WrongType,

    /// When a header should be present, but the key is wrong
    InvalidHeaderKey,

    /// When a header should be present, but the header bytes are wrong
    InvalidHeader,

    /// When the key for the id is wrong
    InvalidIdKey,

    /// When the id is not a valid UTF-8 string
    InvalidId,

    /// When the key for the origin id is wrong
    InvalidOriginIdKey,

    /// When the origin id is not a valid UTF-8 string
    InvalidOriginId,

    /// When the key for the payload is wrong
    InvalidPayloadKey,
}

#[inline]
fn header_is_empty(header: &[u8]) -> bool {
    header.is_empty()
}

/// Represents a response to send whose payload is bytes instead of a specific type
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct UntypedResponse<'a> {
    /// Header data associated with the response as bytes
    #[serde(default, skip_serializing_if = "header_is_empty")]
    pub header: Cow<'a, [u8]>,

    /// Unique id associated with the response
    pub id: Cow<'a, str>,

    /// Unique id associated with the response that triggered the response
    pub origin_id: Cow<'a, str>,

    /// Payload associated with the response as bytes
    pub payload: Cow<'a, [u8]>,
}

impl<'a> UntypedResponse<'a> {
    /// Attempts to convert an untyped response to a typed response
    pub fn to_typed_response<T: DeserializeOwned>(&self) -> io::Result<Response<T>> {
        Ok(Response {
            header: if header_is_empty(&self.header) {
                header!()
            } else {
                utils::deserialize_from_slice(&self.header)?
            },
            id: self.id.to_string(),
            origin_id: self.origin_id.to_string(),
            payload: utils::deserialize_from_slice(&self.payload)?,
        })
    }

    /// Convert into a borrowed version
    pub fn as_borrowed(&self) -> UntypedResponse<'_> {
        UntypedResponse {
            header: match &self.header {
                Cow::Borrowed(x) => Cow::Borrowed(x),
                Cow::Owned(x) => Cow::Borrowed(x.as_slice()),
            },
            id: match &self.id {
                Cow::Borrowed(x) => Cow::Borrowed(x),
                Cow::Owned(x) => Cow::Borrowed(x.as_str()),
            },
            origin_id: match &self.origin_id {
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
    pub fn into_owned(self) -> UntypedResponse<'static> {
        UntypedResponse {
            header: match self.header {
                Cow::Borrowed(x) => Cow::Owned(x.to_vec()),
                Cow::Owned(x) => Cow::Owned(x),
            },
            id: match self.id {
                Cow::Borrowed(x) => Cow::Owned(x.to_string()),
                Cow::Owned(x) => Cow::Owned(x),
            },
            origin_id: match self.origin_id {
                Cow::Borrowed(x) => Cow::Owned(x.to_string()),
                Cow::Owned(x) => Cow::Owned(x),
            },
            payload: match self.payload {
                Cow::Borrowed(x) => Cow::Owned(x.to_vec()),
                Cow::Owned(x) => Cow::Owned(x),
            },
        }
    }

    /// Updates the header of the response to the given `header`.
    pub fn set_header(&mut self, header: impl IntoIterator<Item = u8>) {
        self.header = Cow::Owned(header.into_iter().collect());
    }

    /// Updates the id of the response to the given `id`.
    pub fn set_id(&mut self, id: impl Into<String>) {
        self.id = Cow::Owned(id.into());
    }

    /// Updates the origin id of the response to the given `origin_id`.
    pub fn set_origin_id(&mut self, origin_id: impl Into<String>) {
        self.origin_id = Cow::Owned(origin_id.into());
    }

    /// Allocates a new collection of bytes representing the response.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = vec![];

        let has_header = !header_is_empty(&self.header);
        if has_header {
            rmp::encode::write_map_len(&mut bytes, 4).unwrap();
        } else {
            rmp::encode::write_map_len(&mut bytes, 3).unwrap();
        }

        if has_header {
            rmp::encode::write_str(&mut bytes, "header").unwrap();
            bytes.extend_from_slice(&self.header);
        }

        rmp::encode::write_str(&mut bytes, "id").unwrap();
        rmp::encode::write_str(&mut bytes, &self.id).unwrap();

        rmp::encode::write_str(&mut bytes, "origin_id").unwrap();
        rmp::encode::write_str(&mut bytes, &self.origin_id).unwrap();

        rmp::encode::write_str(&mut bytes, "payload").unwrap();
        bytes.extend_from_slice(&self.payload);

        bytes
    }

    /// Parses a collection of bytes, returning an untyped response if it can be potentially
    /// represented as a [`Response`] depending on the payload.
    ///
    /// NOTE: This supports parsing an invalid response where the payload would not properly
    /// deserialize, but the bytes themselves represent a complete response of some kind.
    pub fn from_slice(input: &'a [u8]) -> Result<Self, UntypedResponseParseError> {
        if input.is_empty() {
            return Err(UntypedResponseParseError::WrongType);
        }

        let has_header = match rmp::Marker::from_u8(input[0]) {
            rmp::Marker::FixMap(3) => false,
            rmp::Marker::FixMap(4) => true,
            _ => return Err(UntypedResponseParseError::WrongType),
        };

        // Advance position by marker
        let input = &input[1..];

        // Parse the header if we have one
        let (header, input) = if has_header {
            let (_, input) = read_key_eq(input, "header")
                .map_err(|_| UntypedResponseParseError::InvalidHeaderKey)?;

            let (header, input) =
                read_header_bytes(input).map_err(|_| UntypedResponseParseError::InvalidHeader)?;
            (header, input)
        } else {
            ([0u8; 0].as_slice(), input)
        };

        // Validate that next field is id
        let (_, input) =
            read_key_eq(input, "id").map_err(|_| UntypedResponseParseError::InvalidIdKey)?;

        // Get the id itself
        let (id, input) =
            read_str_bytes(input).map_err(|_| UntypedResponseParseError::InvalidId)?;

        // Validate that next field is origin_id
        let (_, input) = read_key_eq(input, "origin_id")
            .map_err(|_| UntypedResponseParseError::InvalidOriginIdKey)?;

        // Get the origin_id itself
        let (origin_id, input) =
            read_str_bytes(input).map_err(|_| UntypedResponseParseError::InvalidOriginId)?;

        // Validate that final field is payload
        let (_, input) = read_key_eq(input, "payload")
            .map_err(|_| UntypedResponseParseError::InvalidPayloadKey)?;

        let header = Cow::Borrowed(header);
        let id = Cow::Borrowed(id);
        let origin_id = Cow::Borrowed(origin_id);
        let payload = Cow::Borrowed(input);

        Ok(Self {
            header,
            id,
            origin_id,
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

    // fixstr of 9 bytes with str "origin_id"
    const ORIGIN_ID_FIELD_BYTES: &[u8] =
        &[0xa9, 0x6f, 0x72, 0x69, 0x67, 0x69, 0x6e, 0x5f, 0x69, 0x64];

    // fixstr of 7 bytes with str "payload"
    const PAYLOAD_FIELD_BYTES: &[u8] = &[0xa7, b'p', b'a', b'y', b'l', b'o', b'a', b'd'];

    /// fixstr of 4 bytes with str "test"
    const TEST_STR_BYTES: &[u8] = &[0xa4, b't', b'e', b's', b't'];

    #[test]
    fn untyped_response_should_support_converting_to_bytes() {
        let bytes = Response {
            header: header!(),
            id: "some id".to_string(),
            origin_id: "some origin id".to_string(),
            payload: true,
        }
        .to_vec()
        .unwrap();

        let untyped_response = UntypedResponse::from_slice(&bytes).unwrap();
        assert_eq!(untyped_response.to_bytes(), bytes);
    }

    #[test]
    fn untyped_response_should_support_converting_to_bytes_with_header() {
        let bytes = Response {
            header: header!("key" -> 123),
            id: "some id".to_string(),
            origin_id: "some origin id".to_string(),
            payload: true,
        }
        .to_vec()
        .unwrap();

        let untyped_response = UntypedResponse::from_slice(&bytes).unwrap();
        assert_eq!(untyped_response.to_bytes(), bytes);
    }

    #[test]
    fn untyped_response_should_support_parsing_from_response_bytes_with_header() {
        let bytes = Response {
            header: header!("key" -> 123),
            id: "some id".to_string(),
            origin_id: "some origin id".to_string(),
            payload: true,
        }
        .to_vec()
        .unwrap();

        assert_eq!(
            UntypedResponse::from_slice(&bytes),
            Ok(UntypedResponse {
                header: Cow::Owned(utils::serialize_to_vec(&header!("key" -> 123)).unwrap()),
                id: Cow::Borrowed("some id"),
                origin_id: Cow::Borrowed("some origin id"),
                payload: Cow::Owned(vec![TRUE_BYTE]),
            })
        );
    }

    #[test]
    fn untyped_response_should_support_parsing_from_response_bytes_with_valid_payload() {
        let bytes = Response {
            header: header!(),
            id: "some id".to_string(),
            origin_id: "some origin id".to_string(),
            payload: true,
        }
        .to_vec()
        .unwrap();

        assert_eq!(
            UntypedResponse::from_slice(&bytes),
            Ok(UntypedResponse {
                header: Cow::Owned(vec![]),
                id: Cow::Borrowed("some id"),
                origin_id: Cow::Borrowed("some origin id"),
                payload: Cow::Owned(vec![TRUE_BYTE]),
            })
        );
    }

    #[test]
    fn untyped_response_should_support_parsing_from_response_bytes_with_invalid_payload() {
        // Response with id < 32 bytes
        let mut bytes = Response {
            header: header!(),
            id: "".to_string(),
            origin_id: "".to_string(),
            payload: true,
        }
        .to_vec()
        .unwrap();

        // Push never used byte in msgpack
        bytes.push(NEVER_USED_BYTE);

        // We don't actually check for a valid payload, so the extra byte shows up
        assert_eq!(
            UntypedResponse::from_slice(&bytes),
            Ok(UntypedResponse {
                header: Cow::Owned(vec![]),
                id: Cow::Owned("".to_string()),
                origin_id: Cow::Owned("".to_string()),
                payload: Cow::Owned(vec![TRUE_BYTE, NEVER_USED_BYTE]),
            })
        );
    }

    #[test]
    fn untyped_response_should_support_parsing_full_request() {
        let input = [
            &[0x84],
            HEADER_FIELD_BYTES,
            HEADER_BYTES,
            ID_FIELD_BYTES,
            TEST_STR_BYTES,
            ORIGIN_ID_FIELD_BYTES,
            &[0xa2, b'o', b'g'],
            PAYLOAD_FIELD_BYTES,
            &[TRUE_BYTE],
        ]
        .concat();

        // Convert into typed so we can test
        let untyped_response = UntypedResponse::from_slice(&input).unwrap();
        let response: Response<bool> = untyped_response.to_typed_response().unwrap();

        assert_eq!(response.header, header!("key" -> "value", "num" -> 123));
        assert_eq!(response.id, "test");
        assert_eq!(response.origin_id, "og");
        assert!(response.payload);
    }

    #[test]
    fn untyped_response_should_fail_to_parse_if_given_bytes_not_representing_a_response() {
        // Empty byte slice
        assert_eq!(
            UntypedResponse::from_slice(&[]),
            Err(UntypedResponseParseError::WrongType)
        );

        // Wrong starting byte
        assert_eq!(
            UntypedResponse::from_slice(&[0x00]),
            Err(UntypedResponseParseError::WrongType)
        );

        // Wrong starting byte (fixmap of 0 fields)
        assert_eq!(
            UntypedResponse::from_slice(&[0x80]),
            Err(UntypedResponseParseError::WrongType)
        );

        // Invalid header key
        assert_eq!(
            UntypedResponse::from_slice(
                [
                    &[0x84],
                    &[0xa0], // header key would be defined here, set to empty str
                    HEADER_BYTES,
                    ID_FIELD_BYTES,
                    TEST_STR_BYTES,
                    ORIGIN_ID_FIELD_BYTES,
                    TEST_STR_BYTES,
                    PAYLOAD_FIELD_BYTES,
                    &[TRUE_BYTE],
                ]
                .concat()
                .as_slice()
            ),
            Err(UntypedResponseParseError::InvalidHeaderKey)
        );

        // Invalid header bytes
        assert_eq!(
            UntypedResponse::from_slice(
                [
                    &[0x84],
                    HEADER_FIELD_BYTES,
                    &[0xa0], // header would be defined here, set to empty str
                    ID_FIELD_BYTES,
                    TEST_STR_BYTES,
                    ORIGIN_ID_FIELD_BYTES,
                    TEST_STR_BYTES,
                    PAYLOAD_FIELD_BYTES,
                    &[TRUE_BYTE],
                ]
                .concat()
                .as_slice()
            ),
            Err(UntypedResponseParseError::InvalidHeader)
        );

        // Missing fields (corrupt data)
        assert_eq!(
            UntypedResponse::from_slice(&[0x83]),
            Err(UntypedResponseParseError::InvalidIdKey)
        );

        // Missing id field (has valid data itself)
        assert_eq!(
            UntypedResponse::from_slice(
                [
                    &[0x83],
                    &[0xa0], // id would be defined here, set to empty str
                    TEST_STR_BYTES,
                    ORIGIN_ID_FIELD_BYTES,
                    TEST_STR_BYTES,
                    PAYLOAD_FIELD_BYTES,
                    &[TRUE_BYTE],
                ]
                .concat()
                .as_slice()
            ),
            Err(UntypedResponseParseError::InvalidIdKey)
        );

        // Non-str id field value
        assert_eq!(
            UntypedResponse::from_slice(
                [
                    &[0x83],
                    ID_FIELD_BYTES,
                    &[TRUE_BYTE], // id value set to boolean
                    ORIGIN_ID_FIELD_BYTES,
                    TEST_STR_BYTES,
                    PAYLOAD_FIELD_BYTES,
                    &[TRUE_BYTE],
                ]
                .concat()
                .as_slice()
            ),
            Err(UntypedResponseParseError::InvalidId)
        );

        // Non-utf8 id field value
        assert_eq!(
            UntypedResponse::from_slice(
                [
                    &[0x83],
                    ID_FIELD_BYTES,
                    &[0xa4, 0, 159, 146, 150],
                    ORIGIN_ID_FIELD_BYTES,
                    TEST_STR_BYTES,
                    PAYLOAD_FIELD_BYTES,
                    &[TRUE_BYTE],
                ]
                .concat()
                .as_slice()
            ),
            Err(UntypedResponseParseError::InvalidId)
        );

        // Missing origin_id field (has valid data itself)
        assert_eq!(
            UntypedResponse::from_slice(
                [
                    &[0x83],
                    ID_FIELD_BYTES,
                    TEST_STR_BYTES,
                    &[0xa0], // id would be defined here, set to empty str
                    TEST_STR_BYTES,
                    PAYLOAD_FIELD_BYTES,
                    &[TRUE_BYTE],
                ]
                .concat()
                .as_slice()
            ),
            Err(UntypedResponseParseError::InvalidOriginIdKey)
        );

        // Non-str origin_id field value
        assert_eq!(
            UntypedResponse::from_slice(
                [
                    &[0x83],
                    ID_FIELD_BYTES,
                    TEST_STR_BYTES,
                    ORIGIN_ID_FIELD_BYTES,
                    &[TRUE_BYTE], // id value set to boolean
                    PAYLOAD_FIELD_BYTES,
                    &[TRUE_BYTE],
                ]
                .concat()
                .as_slice()
            ),
            Err(UntypedResponseParseError::InvalidOriginId)
        );

        // Non-utf8 origin_id field value
        assert_eq!(
            UntypedResponse::from_slice(
                [
                    &[0x83],
                    ID_FIELD_BYTES,
                    TEST_STR_BYTES,
                    ORIGIN_ID_FIELD_BYTES,
                    &[0xa4, 0, 159, 146, 150],
                    PAYLOAD_FIELD_BYTES,
                    &[TRUE_BYTE],
                ]
                .concat()
                .as_slice()
            ),
            Err(UntypedResponseParseError::InvalidOriginId)
        );

        // Missing payload field (has valid data itself)
        assert_eq!(
            UntypedResponse::from_slice(
                [
                    &[0x83],
                    ID_FIELD_BYTES,
                    TEST_STR_BYTES,
                    ORIGIN_ID_FIELD_BYTES,
                    TEST_STR_BYTES,
                    &[0xa0], // payload would be defined here, set to empty str
                    &[TRUE_BYTE],
                ]
                .concat()
                .as_slice()
            ),
            Err(UntypedResponseParseError::InvalidPayloadKey)
        );
    }
}
