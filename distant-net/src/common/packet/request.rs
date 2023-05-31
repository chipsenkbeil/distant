use std::borrow::Cow;
use std::{io, str};

use derive_more::{Display, Error};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::{parse_msg_pack_str, write_str_msg_pack, Id};
use crate::common::utils;

/// Represents a request to send
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Request<T> {
    /// Unique id associated with the request
    pub id: Id,

    /// Payload associated with the request
    pub payload: T,
}

impl<T> Request<T> {
    /// Creates a new request with a random, unique id
    pub fn new(payload: T) -> Self {
        Self {
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

    /// When the id is not a valid UTF-8 string
    InvalidId,
}

/// Represents a request to send whose payload is bytes instead of a specific type
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UntypedRequest<'a> {
    /// Unique id associated with the request
    pub id: Cow<'a, str>,

    /// Payload associated with the request as bytes
    pub payload: Cow<'a, [u8]>,
}

impl<'a> UntypedRequest<'a> {
    /// Attempts to convert an untyped request to a typed request
    pub fn to_typed_request<T: DeserializeOwned>(&self) -> io::Result<Request<T>> {
        Ok(Request {
            id: self.id.to_string(),
            payload: utils::deserialize_from_slice(&self.payload)?,
        })
    }

    /// Convert into a borrowed version
    pub fn as_borrowed(&self) -> UntypedRequest<'_> {
        UntypedRequest {
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

    /// Updates the id of the request to the given `id`.
    pub fn set_id(&mut self, id: impl Into<String>) {
        self.id = Cow::Owned(id.into());
    }

    /// Allocates a new collection of bytes representing the request.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = vec![0x82];

        write_str_msg_pack("id", &mut bytes);
        write_str_msg_pack(&self.id, &mut bytes);

        write_str_msg_pack("payload", &mut bytes);
        bytes.extend_from_slice(&self.payload);

        bytes
    }

    /// Parses a collection of bytes, returning a partial request if it can be potentially
    /// represented as a [`Request`] depending on the payload, or the original bytes if it does not
    /// represent a [`Request`]
    ///
    /// NOTE: This supports parsing an invalid request where the payload would not properly
    /// deserialize, but the bytes themselves represent a complete request of some kind.
    pub fn from_slice(input: &'a [u8]) -> Result<Self, UntypedRequestParseError> {
        if input.len() < 2 {
            return Err(UntypedRequestParseError::WrongType);
        }

        // MsgPack marks a fixmap using 0x80 - 0x8f to indicate the size (up to 15 elements).
        //
        // In the case of the request, there are only two elements: id and payload. So the first
        // byte should ALWAYS be 0x82 (130).
        if input[0] != 0x82 {
            return Err(UntypedRequestParseError::WrongType);
        }

        // Skip the first byte representing the fixmap
        let input = &input[1..];

        // Validate that first field is id
        let (input, id_key) =
            parse_msg_pack_str(input).map_err(|_| UntypedRequestParseError::WrongType)?;
        if id_key != "id" {
            return Err(UntypedRequestParseError::WrongType);
        }

        // Get the id itself
        let (input, id) =
            parse_msg_pack_str(input).map_err(|_| UntypedRequestParseError::InvalidId)?;

        // Validate that second field is payload
        let (input, payload_key) =
            parse_msg_pack_str(input).map_err(|_| UntypedRequestParseError::WrongType)?;
        if payload_key != "payload" {
            return Err(UntypedRequestParseError::WrongType);
        }

        let id = Cow::Borrowed(id);
        let payload = Cow::Borrowed(input);

        Ok(Self { id, payload })
    }
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    const TRUE_BYTE: u8 = 0xc3;
    const NEVER_USED_BYTE: u8 = 0xc1;

    // fixstr of 2 bytes with str "id"
    const ID_FIELD_BYTES: &[u8] = &[0xa2, 0x69, 0x64];

    // fixstr of 7 bytes with str "payload"
    const PAYLOAD_FIELD_BYTES: &[u8] = &[0xa7, 0x70, 0x61, 0x79, 0x6c, 0x6f, 0x61, 0x64];

    /// fixstr of 4 bytes with str "test"
    const TEST_STR_BYTES: &[u8] = &[0xa4, 0x74, 0x65, 0x73, 0x74];

    #[test]
    fn untyped_request_should_support_converting_to_bytes() {
        let bytes = Request {
            id: "some id".to_string(),
            payload: true,
        }
        .to_vec()
        .unwrap();

        let untyped_request = UntypedRequest::from_slice(&bytes).unwrap();
        assert_eq!(untyped_request.to_bytes(), bytes);
    }

    #[test]
    fn untyped_request_should_support_parsing_from_request_bytes_with_valid_payload() {
        let bytes = Request {
            id: "some id".to_string(),
            payload: true,
        }
        .to_vec()
        .unwrap();

        assert_eq!(
            UntypedRequest::from_slice(&bytes),
            Ok(UntypedRequest {
                id: Cow::Borrowed("some id"),
                payload: Cow::Owned(vec![TRUE_BYTE]),
            })
        );
    }

    #[test]
    fn untyped_request_should_support_parsing_from_request_bytes_with_invalid_payload() {
        // Request with id < 32 bytes
        let mut bytes = Request {
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
                id: Cow::Owned("".to_string()),
                payload: Cow::Owned(vec![TRUE_BYTE, NEVER_USED_BYTE]),
            })
        );
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

        // Missing fields (corrupt data)
        assert_eq!(
            UntypedRequest::from_slice(&[0x82]),
            Err(UntypedRequestParseError::WrongType)
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
            Err(UntypedRequestParseError::WrongType)
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
            Err(UntypedRequestParseError::WrongType)
        );
    }
}
