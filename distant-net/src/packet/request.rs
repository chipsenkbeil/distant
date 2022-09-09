use super::Id;
use crate::utils;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

/// Represents a request to send
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
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
    pub fn to_vec(&self) -> std::io::Result<Vec<u8>> {
        utils::serialize_to_vec(self)
    }
}

impl<T> Request<T>
where
    T: DeserializeOwned,
{
    /// Deserializes the request from bytes
    pub fn from_slice(slice: &[u8]) -> std::io::Result<Self> {
        utils::deserialize_from_slice(slice)
    }
}

#[cfg(feature = "schemars")]
impl<T: schemars::JsonSchema> Request<T> {
    pub fn root_schema() -> schemars::schema::RootSchema {
        schemars::schema_for!(Request<T>)
    }
}

impl<T> From<T> for Request<T> {
    fn from(payload: T) -> Self {
        Self::new(payload)
    }
}

/// Represents a request to send whose payload is bytes instead of a specific type
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct UntypedRequest {
    /// Unique id associated with the request
    pub id: Id,

    /// Payload associated with the request as bytes
    pub payload: Vec<u8>,
}

impl UntypedRequest {
    /// Parses a collection of bytes, returning a partial request if it can be potentially
    /// represented as a [`Request`] depending on the payload, or the original bytes if it does not
    /// represent a [`Request`]
    ///
    /// NOTE: This supports parsing an invalid request where the payload would not properly
    /// deserialize, but the bytes themselves represent a complete request of some kind.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, Vec<u8>> {
        // MsgPack marks a fixmap using 0x80 - 0x8f to indicate the size (up to 15 elements).
        //
        // In the case of the request, there are only two elements: id and payload. So the first
        // byte should ALWAYS be 0x82 (130).
        todo!();

        // Next, we expect a MsgPack field of some string type to mark the start of the id
        //
        // * fixstr using 0xa0 - 0xbf to mark the start of the id in most cases (where < 32 bytes)
        // * str 8 (0xd9) if up to (2^8)-1 bytes
        // * str 16 (0xda) if up to (2^16)-1 bytes
        // * str 32 (0xdb)  if up to (2^32)-1 bytes

        // Finally, we don't check the remainder of the bytes, so we could be grabbing more or
        // less than expected
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untyped_request_should_support_parsing_from_request_bytes_with_valid_payload() {
        println!(
            "request: {:?}",
            Request {
                id: "hello".to_string(),
                payload: "world"
            }
            .to_vec()
            .unwrap()
        );
        // [130, 162, 105, 100, 180, 49, 52, 56, 52, 51, 52, 57, 55, 57, 54, 50, 56, 57, 51, 55, 48, 56, 53, 50, 54, 167, 112, 97, 121, 108, 111, 97, 100, 164, 116, 101, 115, 116]
        // [130, 162, 105, 100, 165, 104, 101, 108, 108, 111, 167, 112, 97, 121, 108, 111, 97, 100, 165, 119, 111, 114, 108, 100]
        todo!();
    }

    #[test]
    fn untyped_request_should_support_parsing_from_request_bytes_with_invalid_payload() {
        todo!();
    }

    #[test]
    fn untyped_request_should_fail_to_parse_if_given_bytes_not_representing_a_request() {
        todo!();
    }
}
