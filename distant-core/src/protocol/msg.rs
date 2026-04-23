use std::fmt;
use std::marker::PhantomData;

use derive_more::From;
use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

/// Represents a wrapper around a message, supporting single and batch payloads.
#[derive(Clone, Debug, From, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Msg<T> {
    Single(T),
    Batch(Vec<T>),
}

impl<'de, T> Deserialize<'de> for Msg<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct MsgVisitor<T>(PhantomData<T>);

        impl<'de, T> Visitor<'de> for MsgVisitor<T>
        where
            T: Deserialize<'de>,
        {
            type Value = Msg<T>;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(
                    f,
                    "a single {} or a sequence of {} for Msg",
                    std::any::type_name::<T>(),
                    std::any::type_name::<T>(),
                )
            }

            fn visit_seq<A>(self, seq: A) -> Result<Msg<T>, A::Error>
            where
                A: SeqAccess<'de>,
            {
                Vec::<T>::deserialize(SeqAccessDeserializer::new(seq)).map(Msg::Batch)
            }

            fn visit_map<A>(self, map: A) -> Result<Msg<T>, A::Error>
            where
                A: MapAccess<'de>,
            {
                T::deserialize(MapAccessDeserializer::new(map)).map(Msg::Single)
            }
        }

        deserializer.deserialize_any(MsgVisitor::<T>(PhantomData))
    }
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
    pub fn as_mut_single(&mut self) -> Option<&mut T> {
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
    //! Tests for Msg<T>: constructors, accessors (as_single, as_mut_single, as_batch,
    //! as_mut_batch, into_single, into_batch, into_vec), From impls, and serde round-trips.

    use super::*;

    mod single {
        use super::*;

        #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
        struct TestPayload {
            id: u32,
            name: String,
        }

        #[test]
        fn should_be_able_to_serialize_to_json() {
            let msg = Msg::single("hello world");

            let value = serde_json::to_value(msg).unwrap();
            assert_eq!(value, serde_json::json!("hello world"));
        }

        #[test]
        fn should_be_able_to_deserialize_from_json() {
            let value = serde_json::json!({ "id": 1, "name": "hello world" });

            let msg: Msg<TestPayload> = serde_json::from_value(value).unwrap();
            assert_eq!(
                msg,
                Msg::single(TestPayload {
                    id: 1,
                    name: "hello world".into(),
                })
            );
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
            let buf = rmp_serde::encode::to_vec_named(&Msg::single(TestPayload {
                id: 1,
                name: "hello world".into(),
            }))
            .unwrap();

            let msg: Msg<TestPayload> = rmp_serde::decode::from_slice(&buf).unwrap();
            assert_eq!(
                msg,
                Msg::single(TestPayload {
                    id: 1,
                    name: "hello world".into(),
                })
            );
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

    mod constructors_and_accessors {
        use super::*;

        #[test]
        fn single_constructor_should_create_single_variant() {
            let msg: Msg<i32> = Msg::single(42);
            assert!(msg.is_single());
            assert!(!msg.is_batch());
        }

        #[test]
        fn batch_constructor_should_create_batch_variant() {
            let msg: Msg<i32> = Msg::batch([1, 2, 3]);
            assert!(msg.is_batch());
            assert!(!msg.is_single());
        }

        #[test]
        fn batch_constructor_should_accept_iterator() {
            let msg: Msg<i32> = Msg::batch((0..3).map(|i| i * 10));
            assert!(msg.is_batch());
            assert_eq!(msg.as_batch().unwrap(), &[0, 10, 20]);
        }

        #[test]
        fn as_single_should_return_some_for_single() {
            let msg: Msg<i32> = Msg::single(42);
            assert_eq!(msg.as_single(), Some(&42));
        }

        #[test]
        fn as_single_should_return_none_for_batch() {
            let msg: Msg<i32> = Msg::batch([1, 2]);
            assert!(msg.as_single().is_none());
        }

        #[test]
        fn as_mut_single_should_return_some_for_single() {
            let mut msg: Msg<i32> = Msg::single(42);
            assert_eq!(msg.as_mut_single(), Some(&mut 42));
        }

        #[test]
        fn as_mut_single_should_return_none_for_batch() {
            let mut msg: Msg<i32> = Msg::batch([1, 2]);
            assert!(msg.as_mut_single().is_none());
        }

        #[test]
        fn into_single_should_return_some_for_single() {
            let msg: Msg<i32> = Msg::single(42);
            assert_eq!(msg.into_single(), Some(42));
        }

        #[test]
        fn into_single_should_return_none_for_batch() {
            let msg: Msg<i32> = Msg::batch([1, 2]);
            assert!(msg.into_single().is_none());
        }

        #[test]
        fn as_batch_should_return_some_for_batch() {
            let msg: Msg<i32> = Msg::batch([10, 20, 30]);
            assert_eq!(msg.as_batch(), Some([10, 20, 30].as_slice()));
        }

        #[test]
        fn as_batch_should_return_none_for_single() {
            let msg: Msg<i32> = Msg::single(42);
            assert!(msg.as_batch().is_none());
        }

        #[test]
        fn as_mut_batch_should_return_some_for_batch() {
            let mut msg: Msg<i32> = Msg::batch([10, 20]);
            let batch = msg.as_mut_batch().unwrap();
            batch[0] = 99;
            assert_eq!(msg.as_batch().unwrap()[0], 99);
        }

        #[test]
        fn as_mut_batch_should_return_none_for_single() {
            let mut msg: Msg<i32> = Msg::single(42);
            assert!(msg.as_mut_batch().is_none());
        }

        #[test]
        fn into_batch_should_return_some_for_batch() {
            let msg: Msg<i32> = Msg::batch([10, 20]);
            assert_eq!(msg.into_batch(), Some(vec![10, 20]));
        }

        #[test]
        fn into_batch_should_return_none_for_single() {
            let msg: Msg<i32> = Msg::single(42);
            assert!(msg.into_batch().is_none());
        }

        #[test]
        fn into_vec_should_wrap_single_in_vec() {
            let msg: Msg<i32> = Msg::single(42);
            assert_eq!(msg.into_vec(), vec![42]);
        }

        #[test]
        fn into_vec_should_return_batch_contents() {
            let msg: Msg<i32> = Msg::batch([1, 2, 3]);
            assert_eq!(msg.into_vec(), vec![1, 2, 3]);
        }

        #[test]
        fn from_impl_should_create_single_from_value() {
            let msg: Msg<i32> = Msg::from(42);
            assert!(msg.is_single());
            assert_eq!(msg.as_single(), Some(&42));
        }

        #[test]
        fn from_impl_should_create_batch_from_vec() {
            let msg: Msg<i32> = Msg::from(vec![1, 2, 3]);
            assert!(msg.is_batch());
            assert_eq!(msg.as_batch().unwrap(), &[1, 2, 3]);
        }
    }

    mod serde_roundtrips {
        use super::*;

        #[test]
        fn single_json_roundtrip_with_complex_type() {
            #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
            struct Payload {
                id: u32,
                name: String,
            }

            let msg = Msg::single(Payload {
                id: 1,
                name: "test".to_string(),
            });
            let json = serde_json::to_value(&msg).unwrap();
            let restored: Msg<Payload> = serde_json::from_value(json).unwrap();
            assert_eq!(restored, msg);
        }

        #[test]
        fn batch_json_roundtrip_with_complex_type() {
            #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
            struct Payload {
                id: u32,
            }

            let msg = Msg::batch([Payload { id: 1 }, Payload { id: 2 }]);
            let json = serde_json::to_value(&msg).unwrap();
            let restored: Msg<Payload> = serde_json::from_value(json).unwrap();
            assert_eq!(restored, msg);
        }

        #[test]
        fn empty_batch_should_roundtrip_through_json() {
            let msg: Msg<String> = Msg::batch(Vec::<String>::new());
            let json = serde_json::to_value(&msg).unwrap();
            let restored: Msg<String> = serde_json::from_value(json).unwrap();
            assert!(restored.is_batch());
            assert!(restored.as_batch().unwrap().is_empty());
        }
    }

    mod failure_paths {
        use super::*;
        use crate::protocol::{Msg, Request};

        // Mirror production decoration exactly:
        // `Request` / `Response` use `#[serde(rename_all = "snake_case",
        // deny_unknown_fields, tag = "type")]`.
        #[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case", deny_unknown_fields, tag = "type")]
        enum Tagged {
            Alpha { x: u32 },
            Beta { y: String },
        }

        #[test]
        fn single_should_report_inner_error_when_json_deserialize_fails() {
            let json = serde_json::json!({ "type": "gamma" });
            let err = serde_json::from_value::<Msg<Tagged>>(json).unwrap_err();
            let msg = err.to_string();
            // Real error from Tagged must surface — not the untagged collapse.
            assert!(
                msg.contains("gamma"),
                "expected inner error to mention the bad variant `gamma`, got: {msg}"
            );
            assert!(
                !msg.contains("did not match any variant of untagged enum"),
                "expected real error, got untagged-enum collapse: {msg}"
            );
        }

        #[test]
        fn single_should_report_inner_error_when_msgpack_deserialize_fails() {
            // Build a map-shaped msgpack payload with an unknown tag.
            let bytes = rmp_serde::encode::to_vec_named(&serde_json::json!({
                "type": "gamma"
            }))
            .unwrap();
            let err = rmp_serde::decode::from_slice::<Msg<Tagged>>(&bytes).unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("gamma"),
                "expected msgpack inner error mentioning `gamma`, got: {msg}"
            );
            assert!(
                !msg.contains("did not match any variant of untagged enum"),
                "expected real msgpack error, got collapse: {msg}"
            );
        }

        #[test]
        fn single_should_report_inner_error_when_deny_unknown_fields_triggers() {
            // Valid variant but an unknown field — deny_unknown_fields should
            // surface the real error, not the Msg collapse.
            let json = serde_json::json!({
                "type": "alpha",
                "x": 1,
                "extra_field": 2
            });
            let err = serde_json::from_value::<Msg<Tagged>>(json).unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("extra_field") || msg.contains("unknown field"),
                "expected inner error mentioning the unknown field, got: {msg}"
            );
            assert!(
                !msg.contains("did not match any variant of untagged enum"),
                "expected real error, got collapse: {msg}"
            );
        }

        #[test]
        fn single_should_report_inner_error_for_real_request_unknown_variant() {
            // Production path: Msg<protocol::Request> — the exact type the
            // user's pain centers on. Locks down the production path against
            // regressions in `Request`'s serde attributes.
            let json = serde_json::json!({ "type": "mount_xyz_does_not_exist" });
            let err = serde_json::from_value::<Msg<Request>>(json).unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("mount_xyz_does_not_exist"),
                "expected inner error mentioning the bad variant, got: {msg}"
            );
            assert!(
                !msg.contains("did not match any variant of untagged enum"),
                "expected real error, got collapse: {msg}"
            );
        }

        #[test]
        fn batch_should_report_inner_error_when_one_element_fails() {
            // Sub-phase 1's whole point is preserving inner errors — the
            // batch path is half the surface area. A single bad element
            // must still propagate the real error, not the Msg collapse.
            let json = serde_json::json!([
                { "type": "alpha", "x": 1 },
                { "type": "gamma" },
            ]);
            let err = serde_json::from_value::<Msg<Tagged>>(json).unwrap_err();
            let msg = err.to_string();
            assert!(
                msg.contains("gamma"),
                "expected inner error from the failing batch element, got: {msg}"
            );
            assert!(
                !msg.contains("did not match any variant of untagged enum"),
                "expected real error, got collapse: {msg}"
            );
        }

        #[test]
        fn single_should_roundtrip_through_json_with_custom_deserialize() {
            let original: Msg<Tagged> = Msg::Single(Tagged::Alpha { x: 42 });
            let value = serde_json::to_value(&original).unwrap();
            let roundtripped: Msg<Tagged> = serde_json::from_value(value).unwrap();
            assert_eq!(roundtripped, original);
        }

        #[test]
        fn batch_should_roundtrip_through_json_with_custom_deserialize() {
            let original: Msg<Tagged> = Msg::Batch(vec![
                Tagged::Alpha { x: 1 },
                Tagged::Beta { y: "two".into() },
            ]);
            let value = serde_json::to_value(&original).unwrap();
            let roundtripped: Msg<Tagged> = serde_json::from_value(value).unwrap();
            assert_eq!(roundtripped, original);
        }

        #[test]
        fn batch_should_roundtrip_through_msgpack_with_custom_deserialize() {
            let original: Msg<Tagged> = Msg::Batch(vec![
                Tagged::Alpha { x: 1 },
                Tagged::Beta { y: "two".into() },
            ]);
            let bytes = rmp_serde::encode::to_vec_named(&original).unwrap();
            let roundtripped: Msg<Tagged> = rmp_serde::decode::from_slice(&bytes).unwrap();
            assert_eq!(roundtripped, original);
        }
    }
}
