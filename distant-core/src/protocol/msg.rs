use derive_more::From;
use serde::{Deserialize, Serialize};

/// Represents a wrapper around a message, supporting single and batch payloads.
#[derive(Clone, Debug, From, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Msg<T> {
    Single(T),
    Batch(Vec<T>),
}

impl<T> Msg<T> {
    /// Creates a new msg with a singular payload.
    #[inline]
    pub fn single(payload: T) -> Self {
        Self::Single(payload)
    }

    /// Creates a new msg with a batch payload.
    pub fn batch<I>(payloads: I) -> Self
    where
        I: IntoIterator<Item = T>,
    {
        Self::Batch(payloads.into_iter().collect())
    }

    /// Returns true if msg has a single payload.
    #[inline]
    pub fn is_single(&self) -> bool {
        matches!(self, Self::Single(_))
    }

    /// Returns reference to single value if msg is single variant.
    #[inline]
    pub fn as_single(&self) -> Option<&T> {
        match self {
            Self::Single(x) => Some(x),
            _ => None,
        }
    }

    /// Returns mutable reference to single value if msg is single variant.
    #[inline]
    pub fn as_mut_single(&mut self) -> Option<&T> {
        match self {
            Self::Single(x) => Some(x),
            _ => None,
        }
    }

    /// Returns the single value if msg is single variant.
    #[inline]
    pub fn into_single(self) -> Option<T> {
        match self {
            Self::Single(x) => Some(x),
            _ => None,
        }
    }

    /// Returns true if msg has a batch of payloads.
    #[inline]
    pub fn is_batch(&self) -> bool {
        matches!(self, Self::Batch(_))
    }

    /// Returns reference to batch value if msg is batch variant.
    #[inline]
    pub fn as_batch(&self) -> Option<&[T]> {
        match self {
            Self::Batch(x) => Some(x),
            _ => None,
        }
    }

    /// Returns mutable reference to batch value if msg is batch variant.
    #[inline]
    pub fn as_mut_batch(&mut self) -> Option<&mut [T]> {
        match self {
            Self::Batch(x) => Some(x),
            _ => None,
        }
    }

    /// Returns the batch value if msg is batch variant.
    #[inline]
    pub fn into_batch(self) -> Option<Vec<T>> {
        match self {
            Self::Batch(x) => Some(x),
            _ => None,
        }
    }

    /// Convert into a collection of payload data.
    #[inline]
    pub fn into_vec(self) -> Vec<T> {
        match self {
            Self::Single(x) => vec![x],
            Self::Batch(x) => x,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod single {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let msg = Msg::single("hello world");

            let value = serde_json::to_value(msg).unwrap();
            assert_eq!(value, serde_json::json!("hello world"));
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!("hello world");

            let msg: Msg<String> = serde_json::from_value(value).unwrap();
            assert_eq!(msg, Msg::single(String::from("hello world")));
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let msg = Msg::single("hello world");

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&msg).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Msg::single("hello world")).unwrap();

            let msg: Msg<String> = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(msg, Msg::single(String::from("hello world")));
        }
    }

    mod batch {
        use super::*;

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let msg = Msg::batch(["hello world"]);

            let value = serde_json::to_value(msg).unwrap();
            assert_eq!(value, serde_json::json!(["hello world"]));
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!(["hello world"]);

            let msg: Msg<String> = serde_json::from_value(value).unwrap();
            assert_eq!(msg, Msg::batch([String::from("hello world")]));
        }

        #[test]
        fn should_be_able_to_serialize_to_msgpack() {
            let msg = Msg::batch(["hello world"]);

            // NOTE: We don't actually check the output here because it's an implementation detail
            // and could change as we change how serialization is done. This is merely to verify
            // that we can serialize since there are times when serde fails to serialize at
            // runtime.
            let _ = rmp_serde::encode::to_vec_named(&msg).unwrap();
        }

        #[test]
        fn should_be_able_to_deserialize_from_msgpack() {
            // NOTE: It may seem odd that we are serializing just to deserialize, but this is to
            // verify that we are not corrupting or causing issues when serializing on a
            // client/server and then trying to deserialize on the other side. This has happened
            // enough times with minor changes that we need tests to verify.
            let buf = rmp_serde::encode::to_vec_named(&Msg::batch(["hello world"])).unwrap();

            let msg: Msg<String> = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(msg, Msg::batch([String::from("hello world")]));
        }
    }
}
