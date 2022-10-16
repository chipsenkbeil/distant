use super::{parse_msg_pack_str, write_str_msg_pack, Id};
use crate::common::utils;
use derive_more::{Display, Error};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{borrow::Cow, io};

/// Represents a response received related to some response
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct Response<T> {
    /// Unique id associated with the response
    pub id: Id,

    /// Unique id associated with the response that triggered the response
    pub origin_id: Id,

    /// Payload associated with the response
    pub payload: T,
}

impl<T> Response<T> {
    /// Creates a new response with a random, unique id
    pub fn new(origin_id: Id, payload: T) -> Self {
        Self {
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

#[cfg(feature = "schemars")]
impl<T: schemars::JsonSchema> Response<T> {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(Response<T>)
    }
}

/// Error encountered when attempting to parse bytes as an untyped response
#[derive(Copy, Clone, Debug, Display, Error, PartialEq, Eq, Hash)]
pub enum UntypedResponseParseError {
    /// When the bytes do not represent a response
    WrongType,

    /// When the id is not a valid UTF-8 string
    InvalidId,

    /// When the origin id is not a valid UTF-8 string
    InvalidOriginId,
}

/// Represents a response to send whose payload is bytes instead of a specific type
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct UntypedResponse<'a> {
    /// Unique id associated with the response
    pub id: Cow<'a, str>,

    /// Unique id associated with the response that triggered the response
    pub origin_id: Cow<'a, str>,

    /// Payload associated with the response as bytes
    pub payload: Cow<'a, [u8]>,
}

impl<'a> UntypedResponse<'a> {
    /// Attempts to convert an untyped request to a typed request
    pub fn to_typed_response<T: DeserializeOwned>(&self) -> io::Result<Response<T>> {
        Ok(Response {
            id: self.id.to_string(),
            origin_id: self.origin_id.to_string(),
            payload: utils::deserialize_from_slice(&self.payload)?,
        })
    }

    /// Convert into a borrowed version
    pub fn as_borrowed(&self) -> UntypedResponse<'_> {
        UntypedResponse {
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
        let mut bytes = vec![0x83];

        write_str_msg_pack("id", &mut bytes);
        write_str_msg_pack(&self.id, &mut bytes);

        write_str_msg_pack("origin_id", &mut bytes);
        write_str_msg_pack(&self.origin_id, &mut bytes);

        write_str_msg_pack("payload", &mut bytes);
        bytes.extend_from_slice(&self.payload);

        bytes
    }

    /// Parses a collection of bytes, returning an untyped response if it can be potentially
    /// represented as a [`Response`] depending on the payload, or the original bytes if it does not
    /// represent a [`Response`].
    ///
    /// NOTE: This supports parsing an invalid response where the payload would not properly
    /// deserialize, but the bytes themselves represent a complete response of some kind.
    pub fn from_slice(input: &'a [u8]) -> Result<Self, UntypedResponseParseError> {
        if input.len() < 2 {
            return Err(UntypedResponseParseError::WrongType);
        }

        // MsgPack marks a fixmap using 0x80 - 0x8f to indicate the size (up to 15 elements).
        //
        // In the case of the request, there are only three elements: id, origin_id, and payload.
        // So the first byte should ALWAYS be 0x83 (131).
        if input[0] != 0x83 {
            return Err(UntypedResponseParseError::WrongType);
        }

        // Skip the first byte representing the fixmap
        let input = &input[1..];

        // Validate that first field is id
        let (input, id_key) =
            parse_msg_pack_str(input).map_err(|_| UntypedResponseParseError::WrongType)?;
        if id_key != "id" {
            return Err(UntypedResponseParseError::WrongType);
        }

        // Get the id itself
        let (input, id) =
            parse_msg_pack_str(input).map_err(|_| UntypedResponseParseError::InvalidId)?;

        // Validate that second field is origin_id
        let (input, origin_id_key) =
            parse_msg_pack_str(input).map_err(|_| UntypedResponseParseError::WrongType)?;
        if origin_id_key != "origin_id" {
            return Err(UntypedResponseParseError::WrongType);
        }

        // Get the origin_id itself
        let (input, origin_id) =
            parse_msg_pack_str(input).map_err(|_| UntypedResponseParseError::InvalidOriginId)?;

        // Validate that second field is payload
        let (input, payload_key) =
            parse_msg_pack_str(input).map_err(|_| UntypedResponseParseError::WrongType)?;
        if payload_key != "payload" {
            return Err(UntypedResponseParseError::WrongType);
        }

        let id = Cow::Borrowed(id);
        let origin_id = Cow::Borrowed(origin_id);
        let payload = Cow::Borrowed(input);

        Ok(Self {
            id,
            origin_id,
            payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    const TRUE_BYTE: u8 = 0xc3;
    const NEVER_USED_BYTE: u8 = 0xc1;

    // fixstr of 2 bytes with str "id"
    const ID_FIELD_BYTES: &[u8] = &[0xa2, 0x69, 0x64];

    // fixstr of 9 bytes with str "origin_id"
    const ORIGIN_ID_FIELD_BYTES: &[u8] =
        &[0xa9, 0x6f, 0x72, 0x69, 0x67, 0x69, 0x6e, 0x5f, 0x69, 0x64];

    // fixstr of 7 bytes with str "payload"
    const PAYLOAD_FIELD_BYTES: &[u8] = &[0xa7, 0x70, 0x61, 0x79, 0x6c, 0x6f, 0x61, 0x64];

    /// fixstr of 4 bytes with str "test"
    const TEST_STR_BYTES: &[u8] = &[0xa4, 0x74, 0x65, 0x73, 0x74];

    #[test]
    fn untyped_response_should_support_converting_to_bytes() {
        let bytes = Response {
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
    fn untyped_response_should_support_parsing_from_response_bytes_with_valid_payload() {
        let bytes = Response {
            id: "some id".to_string(),
            origin_id: "some origin id".to_string(),
            payload: true,
        }
        .to_vec()
        .unwrap();

        assert_eq!(
            UntypedResponse::from_slice(&bytes),
            Ok(UntypedResponse {
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
                id: Cow::Owned("".to_string()),
                origin_id: Cow::Owned("".to_string()),
                payload: Cow::Owned(vec![TRUE_BYTE, NEVER_USED_BYTE]),
            })
        );
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

        // Missing fields (corrupt data)
        assert_eq!(
            UntypedResponse::from_slice(&[0x83]),
            Err(UntypedResponseParseError::WrongType)
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
            Err(UntypedResponseParseError::WrongType)
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
            Err(UntypedResponseParseError::WrongType)
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
            Err(UntypedResponseParseError::WrongType)
        );
    }
}
