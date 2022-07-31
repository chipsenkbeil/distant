/// Represents a generic id type
pub type Id = String;

/// Represents a request to send
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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

impl<T> From<T> for Request<T> {
    fn from(payload: T) -> Self {
        Self::new(payload)
    }
}

/// Represents a response received related to some request
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Response<T> {
    /// Unique id associated with the response
    pub id: Id,

    /// Unique id associated with the request that triggered the response
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
