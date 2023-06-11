use std::borrow::Cow;
use std::{io, str};

use derive_more::{Display, Error};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::{read_header_bytes, read_str_bytes, Header, Id};
use crate::common::utils;
use crate::header;

/// Represents a request to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request<T> {
    /// Optional header data to include with request.
    pub header: Header,

    /// Unique id associated with the request.
    pub id: Id,

    /// Payload associated with the request.
    pub payload: T,
}

impl<T> Request<T> {
    /// Creates a new request with a random, unique id and no header data.
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
    /// Serializes the request into bytes using a compact approach.
    pub fn to_vec(&self) -> io::Result<Vec<u8>> {
        Ok(self.to_untyped_request()?.to_bytes())
    }

    /// Serializes the request's payload into bytes.
    pub fn to_payload_vec(&self) -> io::Result<Vec<u8>> {
        utils::serialize_to_vec(&self.payload)
    }

    /// Attempts to convert a typed request to an untyped request.
    pub fn to_untyped_request(&self) -> io::Result<UntypedRequest> {
        Ok(UntypedRequest {
            header: Cow::Owned(if !self.header.is_empty() {
                self.header.to_vec()?
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
    /// Deserializes the request from bytes.
    pub fn from_slice(slice: &[u8]) -> io::Result<Self> {
        UntypedRequest::from_slice(slice)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?
            .to_typed_request()
    }
}

impl<T> From<T> for Request<T> {
    fn from(payload: T) -> Self {
        Self::new(payload)
    }
}

mod serde_impl {
    use super::*;
    use serde::de::{self, Deserialize, Deserializer, SeqAccess, Visitor};
    use serde::ser::{Serialize, SerializeSeq, Serializer};
    use std::fmt;
    use std::marker::PhantomData;

    impl<T> Serialize for Request<T>
    where
        T: Serialize,
    {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            let has_header = !self.header.is_empty();
            let mut cnt = 2;
            if has_header {
                cnt += 1;
            }

            let mut seq = serializer.serialize_seq(Some(cnt))?;

            if has_header {
                seq.serialize_element(&self.header)?;
            }
            seq.serialize_element(&self.id)?;
            seq.serialize_element(&self.payload)?;
            seq.end()
        }
    }

    impl<'de, T> Deserialize<'de> for Request<T>
    where
        T: Deserialize<'de>,
    {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_seq(RequestVisitor::new())
        }
    }

    struct RequestVisitor<T> {
        marker: PhantomData<fn() -> Request<T>>,
    }

    impl<T> RequestVisitor<T> {
        fn new() -> Self {
            Self {
                marker: PhantomData,
            }
        }
    }

    impl<'de, T> Visitor<'de> for RequestVisitor<T>
    where
        T: Deserialize<'de>,
    {
        type Value = Request<T>;

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
                Some(2) => Header::default(),
                Some(3) => access
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
            let payload = access
                .next_element()?
                .ok_or_else(|| de::Error::custom("missing payload"))?;

            Ok(Request {
                header,
                id,
                payload,
            })
        }
    }
}

/// Error encountered when attempting to parse bytes as an untyped request.
#[derive(Copy, Clone, Debug, Display, Error, PartialEq, Eq, Hash)]
pub enum UntypedRequestParseError {
    /// When the bytes do not represent a request.
    WrongType,

    /// When a header should be present, but the header bytes are wrong.
    InvalidHeader,

    /// When the id is not a valid UTF-8 string.
    InvalidId,

    /// When no payload found in the request.
    MissingPayload,
}

#[inline]
fn header_is_empty(header: &[u8]) -> bool {
    header.is_empty()
}

/// Represents a request to send whose payload is bytes instead of a specific type.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UntypedRequest<'a> {
    /// Header data associated with the request as bytes.
    #[serde(default, skip_serializing_if = "header_is_empty")]
    pub header: Cow<'a, [u8]>,

    /// Unique id associated with the request.
    pub id: Cow<'a, str>,

    /// Payload associated with the request as bytes.
    pub payload: Cow<'a, [u8]>,
}

impl<'a> UntypedRequest<'a> {
    /// Attempts to convert an untyped request to a typed request.
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

    /// Convert into a borrowed version.
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

    /// Convert into an owned version.
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

    /// Returns true if the request has an empty header.
    #[inline]
    pub fn is_header_empty(&self) -> bool {
        header_is_empty(&self.header)
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
            rmp::encode::write_array_len(&mut bytes, 3).unwrap();
        } else {
            rmp::encode::write_array_len(&mut bytes, 2).unwrap();
        }

        if has_header {
            bytes.extend_from_slice(&self.header);
        }

        rmp::encode::write_str(&mut bytes, &self.id).unwrap();
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
            rmp::Marker::FixArray(2) => false,
            rmp::Marker::FixArray(3) => true,
            _ => return Err(UntypedRequestParseError::WrongType),
        };

        // Advance position by marker
        let input = &input[1..];

        // Parse the header if we have one
        let (header, input) = if has_header {
            read_header_bytes(input).map_err(|_| UntypedRequestParseError::InvalidHeader)?
        } else {
            ([0u8; 0].as_slice(), input)
        };

        let (id, input) = read_str_bytes(input).map_err(|_| UntypedRequestParseError::InvalidId)?;

        // Check if we have input remaining, which should be our payload
        if input.is_empty() {
            return Err(UntypedRequestParseError::MissingPayload);
        }

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
    fn untyped_request_should_support_parsing_without_header() {
        let input = [
            &[rmp::Marker::FixArray(2).to_u8()],
            TEST_STR_BYTES,
            &[TRUE_BYTE],
        ]
        .concat();

        // Convert into typed so we can test
        let untyped_request = UntypedRequest::from_slice(&input).unwrap();
        let request: Request<bool> = untyped_request.to_typed_request().unwrap();

        assert_eq!(request.header, header!());
        assert_eq!(request.id, "test");
        assert!(request.payload);
    }

    #[test]
    fn untyped_request_should_support_parsing_full_request() {
        let input = [
            &[rmp::Marker::FixArray(3).to_u8()],
            HEADER_BYTES,
            TEST_STR_BYTES,
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

        // Wrong starting byte (fixarray of 0 fields)
        assert_eq!(
            UntypedRequest::from_slice(&[rmp::Marker::FixArray(0).to_u8()]),
            Err(UntypedRequestParseError::WrongType)
        );

        // Missing fields (corrupt data)
        assert_eq!(
            UntypedRequest::from_slice(&[rmp::Marker::FixArray(2).to_u8()]),
            Err(UntypedRequestParseError::InvalidId)
        );

        // Missing fields (corrupt data)
        assert_eq!(
            UntypedRequest::from_slice(&[rmp::Marker::FixArray(3).to_u8()]),
            Err(UntypedRequestParseError::InvalidHeader)
        );

        // Invalid header bytes
        assert_eq!(
            UntypedRequest::from_slice(
                [
                    &[rmp::Marker::FixArray(3).to_u8()],
                    &[0xa0], // header would be defined here, set to empty str
                    TEST_STR_BYTES,
                    &[TRUE_BYTE],
                ]
                .concat()
                .as_slice()
            ),
            Err(UntypedRequestParseError::InvalidHeader)
        );

        // Non-str id field value
        assert_eq!(
            UntypedRequest::from_slice(&[
                rmp::Marker::FixArray(2).to_u8(),
                TRUE_BYTE, // id value set to boolean
                TRUE_BYTE,
            ]),
            Err(UntypedRequestParseError::InvalidId)
        );

        // Non-utf8 id field value
        assert_eq!(
            UntypedRequest::from_slice(&[
                rmp::Marker::FixArray(2).to_u8(),
                0xa4,
                0,
                159,
                146,
                150,
                TRUE_BYTE,
            ]),
            Err(UntypedRequestParseError::InvalidId)
        );

        // Missing payload
        assert_eq!(
            UntypedRequest::from_slice(
                [&[rmp::Marker::FixArray(2).to_u8()], TEST_STR_BYTES]
                    .concat()
                    .as_slice()
            ),
            Err(UntypedRequestParseError::MissingPayload)
        );
    }
}
