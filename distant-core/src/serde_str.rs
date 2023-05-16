use std::fmt;
use std::marker::PhantomData;
use std::str::FromStr;

use serde::de::{Deserializer, Error as SerdeError, Visitor};
use serde::ser::Serializer;

/// From https://docs.rs/serde_with/1.14.0/src/serde_with/rust.rs.html#90-118
pub fn deserialize_from_str<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: fmt::Display,
{
    struct Helper<S>(PhantomData<S>);

    impl<'de, S> Visitor<'de> for Helper<S>
    where
        S: FromStr,
        <S as FromStr>::Err: fmt::Display,
    {
        type Value = S;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "a string")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: SerdeError,
        {
            value.parse::<Self::Value>().map_err(SerdeError::custom)
        }
    }

    deserializer.deserialize_str(Helper(PhantomData))
}

/// From https://docs.rs/serde_with/1.14.0/src/serde_with/rust.rs.html#121-127
pub fn serialize_to_str<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
where
    T: fmt::Display,
    S: Serializer,
{
    serializer.collect_str(&value)
}
