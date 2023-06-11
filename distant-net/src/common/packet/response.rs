use std::borrow::Cow;
use std::io;

use derive_more::{Display, Error};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::{read_header_bytes, read_str_bytes, Header, Id};
use crate::common::utils;
use crate::header;

/// Represents a response received related to some response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Response<T> {
    /// Optional header data to include with response.
    pub header: Header,

    /// Unique id associated with the response.
    pub id: Id,

    /// Unique id associated with the response that triggered the response.
    pub origin_id: Id,

    /// Payload associated with the response.
    pub payload: T,
}

impl<T> Response<T> {
    /// Creates a new response with a random, unique id and no header data.
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
    /// Serializes the response into bytes using a compact approach.
    pub fn to_vec(&self) -> std::io::Result<Vec<u8>> {
        Ok(self.to_untyped_response()?.to_bytes())
    }

    /// Serializes the response's payload into bytes.
    pub fn to_payload_vec(&self) -> io::Result<Vec<u8>> {
        utils::serialize_to_vec(&self.payload)
    }

    /// Attempts to convert a typed response to an untyped response.
    pub fn to_untyped_response(&self) -> io::Result<UntypedResponse> {
        Ok(UntypedResponse {
            header: Cow::Owned(if !self.header.is_empty() {
                self.header.to_vec()?
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
    /// Deserializes the response from bytes.
    pub fn from_slice(slice: &[u8]) -> std::io::Result<Self> {
        UntypedResponse::from_slice(slice)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?
            .to_typed_response()
    }
}

mod serde_impl {
    use super::*;
    use serde::de::{self, Deserialize, Deserializer, SeqAccess, Visitor};
    use serde::ser::{Serialize, SerializeSeq, Serializer};
    use std::fmt;
    use std::marker::PhantomData;

    impl<T> Serialize for Response<T>
    where
        T: Serialize,
    {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let has_header = !self.header.is_empty();
            let mut cnt = 3;
            if has_header {
                cnt += 1;
            }

            let mut seq = serializer.serialize_seq(Some(cnt))?;

            if has_header {
                seq.serialize_element(&self.header)?;
            }
            seq.serialize_element(&self.id)?;
            seq.serialize_element(&self.origin_id)?;
            seq.serialize_element(&self.payload)?;
            seq.end()
        }
    }

    impl<'de, T> Deserialize<'de> for Response<T>
    where
        T: Deserialize<'de>,
    {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_seq(ResponseVisitor::new())
        }
    }

    struct ResponseVisitor<T> {
        marker: PhantomData<fn() -> Response<T>>,
    }

    impl<T> ResponseVisitor<T> {
        fn new() -> Self {
            Self {
                marker: PhantomData,
            }
        }
    }

    impl<'de, T> Visitor<'de> for ResponseVisitor<T>
    where
        T: Deserialize<'de>,
    {
        type Value = Response<T>;

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("a response")
        }

        fn visit_seq<S>(self, mut access: S) -> Result<Self::Value, S::Error>
        where
            S: SeqAccess<'de>,
        {
            // Attempt to determine if we have a header based on size, defaulting to attempting
            // to parse the header first if we don't know the size. If we cannot parse the header,
            // we use the default header and keep going.
            let header = match access.size_hint() {
                Some(3) => Header::default(),
                Some(4) => access
                    .next_element()?
                    .ok_or_else(|| de::Error::custom("missing header"))?,
                Some(_) => return Err(de::Error::custom("invalid response array len")),
                None => access
                    .next_element::<Header>()
                    .ok()
                    .flatten()
                    .unwrap_or_default(),
            };

            let id = access
                .next_element()?
                .ok_or_else(|| de::Error::custom("missing id"))?;
            let origin_id = access
                .next_element()?
                .ok_or_else(|| de::Error::custom("missing origin_id"))?;
            let payload = access
                .next_element()?
                .ok_or_else(|| de::Error::custom("missing payload"))?;

            Ok(Response {
                header,
                id,
                origin_id,
                payload,
            })
        }
    }
}

/// Error encountered when attempting to parse bytes as an untyped response.
#[derive(Copy, Clone, Debug, Display, Error, PartialEq, Eq, Hash)]
pub enum UntypedResponseParseError {
    /// When the bytes do not represent a response.
    WrongType,

    /// When a header should be present, but the header bytes are wrong.
    InvalidHeader,

    /// When the id is not a valid UTF-8 string.
    InvalidId,

    /// When the origin id is not a valid UTF-8 string.
    InvalidOriginId,

    /// When no payload found in the request.
    MissingPayload,
}

#[inline]
fn header_is_empty(header: &[u8]) -> bool {
    header.is_empty()
}

/// Represents a response to send whose payload is bytes instead of a specific type.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct UntypedResponse<'a> {
    /// Header data associated with the response as bytes.
    #[serde(default, skip_serializing_if = "header_is_empty")]
    pub header: Cow<'a, [u8]>,

    /// Unique id associated with the response.
    pub id: Cow<'a, str>,

    /// Unique id associated with the response that triggered the response.
    pub origin_id: Cow<'a, str>,

    /// Payload associated with the response as bytes.
    pub payload: Cow<'a, [u8]>,
}

impl<'a> UntypedResponse<'a> {
    /// Attempts to convert an untyped response to a typed response.
    pub fn to_typed_response<T: DeserializeOwned>(&self) -> io::Result<Response<T>> {
        Ok(Response {
            header: if header_is_empty(&self.header) {
                header!()
            } else {
                Header::from_slice(&self.header)?
            },
            id: self.id.to_string(),
            origin_id: self.origin_id.to_string(),
            payload: utils::deserialize_from_slice(&self.payload)?,
        })
    }

    /// Convert into a borrowed version.
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

    /// Convert into an owned version.
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
            rmp::encode::write_array_len(&mut bytes, 4).unwrap();
        } else {
            rmp::encode::write_array_len(&mut bytes, 3).unwrap();
        }

        if has_header {
            bytes.extend_from_slice(&self.header);
        }

        rmp::encode::write_str(&mut bytes, &self.id).unwrap();
        rmp::encode::write_str(&mut bytes, &self.origin_id).unwrap();
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
            rmp::Marker::FixArray(3) => false,
            rmp::Marker::FixArray(4) => true,
            _ => return Err(UntypedResponseParseError::WrongType),
        };

        // Advance position by marker
        let input = &input[1..];

        // Parse the header if we have one
        let (header, input) = if has_header {
            read_header_bytes(input).map_err(|_| UntypedResponseParseError::InvalidHeader)?
        } else {
            ([0u8; 0].as_slice(), input)
        };

        let (id, input) =
            read_str_bytes(input).map_err(|_| UntypedResponseParseError::InvalidId)?;

        let (origin_id, input) =
            read_str_bytes(input).map_err(|_| UntypedResponseParseError::InvalidOriginId)?;

        // Check if we have input remaining, which should be our payload
        if input.is_empty() {
            return Err(UntypedResponseParseError::MissingPayload);
        }

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
    fn untyped_response_should_support_parsing_without_header() {
        let input = [
            &[rmp::Marker::FixArray(3).to_u8()],
            TEST_STR_BYTES,
            &[0xa2, b'o', b'g'],
            &[TRUE_BYTE],
        ]
        .concat();

        // Convert into typed so we can test
        let untyped_response = UntypedResponse::from_slice(&input).unwrap();
        let response: Response<bool> = untyped_response.to_typed_response().unwrap();

        assert_eq!(response.header, header!());
        assert_eq!(response.id, "test");
        assert_eq!(response.origin_id, "og");
        assert!(response.payload);
    }

    #[test]
    fn untyped_response_should_support_parsing_full_request() {
        let input = [
            &[rmp::Marker::FixArray(4).to_u8()],
            HEADER_BYTES,
            TEST_STR_BYTES,
            &[0xa2, b'o', b'g'],
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

        // Missing fields (corrupt data)
        assert_eq!(
            UntypedResponse::from_slice(&[rmp::Marker::FixArray(3).to_u8()]),
            Err(UntypedResponseParseError::InvalidId)
        );

        // Missing fields (corrupt data)
        assert_eq!(
            UntypedResponse::from_slice(&[rmp::Marker::FixArray(4).to_u8()]),
            Err(UntypedResponseParseError::InvalidHeader)
        );

        // Invalid header bytes
        assert_eq!(
            UntypedResponse::from_slice(
                [
                    &[rmp::Marker::FixArray(4).to_u8()],
                    &[0xa0], // header would be defined here, set to empty str
                    TEST_STR_BYTES,
                    TEST_STR_BYTES,
                    &[TRUE_BYTE],
                ]
                .concat()
                .as_slice()
            ),
            Err(UntypedResponseParseError::InvalidHeader)
        );

        // Non-str id field value
        assert_eq!(
            UntypedResponse::from_slice(
                [
                    &[rmp::Marker::FixArray(3).to_u8()],
                    &[TRUE_BYTE], // id value set to boolean
                    TEST_STR_BYTES,
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
                    &[rmp::Marker::FixArray(3).to_u8()] as &[u8],
                    &[0xa4, 0, 159, 146, 150],
                    TEST_STR_BYTES,
                    &[TRUE_BYTE],
                ]
                .concat()
                .as_slice()
            ),
            Err(UntypedResponseParseError::InvalidId)
        );

        // Non-str origin_id field value
        assert_eq!(
            UntypedResponse::from_slice(
                [
                    &[rmp::Marker::FixArray(3).to_u8()],
                    TEST_STR_BYTES,
                    &[TRUE_BYTE], // id value set to boolean
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
                    &[rmp::Marker::FixArray(3).to_u8()],
                    TEST_STR_BYTES,
                    &[0xa4, 0, 159, 146, 150],
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
                    &[rmp::Marker::FixArray(3).to_u8()],
                    TEST_STR_BYTES,
                    TEST_STR_BYTES,
                ]
                .concat()
                .as_slice()
            ),
            Err(UntypedResponseParseError::MissingPayload)
        );
    }
}
