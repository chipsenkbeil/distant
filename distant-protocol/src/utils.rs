use serde::{Deserialize, Serialize};

/// Used purely for skipping serialization of values that are false by default.
#[inline]
pub const fn is_false(value: &bool) -> bool {
    !*value
}

/// Used purely for skipping serialization of values that are 1 by default.
#[inline]
pub const fn is_one(value: &usize) -> bool {
    *value == 1
}

/// Used to provide a default serde value of 1.
#[inline]
pub const fn one() -> usize {
    1
}

pub fn deserialize_u128_option<'de, D>(deserializer: D) -> Result<Option<u128>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match Option::<String>::deserialize(deserializer)? {
        Some(s) => match s.parse::<u128>() {
            Ok(value) => Ok(Some(value)),
            Err(error) => Err(serde::de::Error::custom(format!(
                "Cannot convert to u128 with error: {error:?}"
            ))),
        },
        None => Ok(None),
    }
}

pub fn serialize_u128_option<S: serde::Serializer>(
    val: &Option<u128>,
    s: S,
) -> Result<S::Ok, S::Error> {
    match val {
        Some(v) => format!("{}", *v).serialize(s),
        None => s.serialize_unit(),
    }
}
