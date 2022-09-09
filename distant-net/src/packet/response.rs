use super::Id;
use crate::utils;
use serde::{de::DeserializeOwned, Deserialize, Serialize};

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

/// Represents a response to send whose payload is bytes instead of a specific type
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct PartialResponse {
    /// Unique id associated with the response
    pub id: Id,

    /// Unique id associated with the response that triggered the response
    pub origin_id: Id,

    /// Payload associated with the response as bytes
    pub payload: Vec<u8>,
}

impl PartialResponse {
    /// Parses a collection of bytes, returning a partial response if it can be potentially
    /// represented as a [`Response`] depending on the payload, or the original bytes if it does not
    /// represent a [`Response`]
    ///
    /// NOTE: This supports parsing an invalid response where the payload would not properly
    /// deserialize, but the bytes themselves represent a complete response of some kind.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, Vec<u8>> {
        todo!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partial_response_should_support_parsing_from_response_bytes_with_valid_payload() {
        todo!();
    }

    #[test]
    fn partial_response_should_support_parsing_from_response_bytes_with_invalid_payload() {
        todo!();
    }

    #[test]
    fn partial_response_should_fail_to_parse_if_given_bytes_not_representing_a_response() {
        todo!();
    }
}
