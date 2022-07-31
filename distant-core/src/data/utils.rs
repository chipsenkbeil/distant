use serde::{Deserialize, Serialize};

pub(crate) fn deserialize_u128_option<'de, D>(deserializer: D) -> Result<Option<u128>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    match Option::<String>::deserialize(deserializer)? {
        Some(s) => match s.parse::<u128>() {
            Ok(value) => Ok(Some(value)),
            Err(error) => Err(serde::de::Error::custom(format!(
                "Cannot convert to u128 with error: {:?}",
                error
            ))),
        },
        None => Ok(None),
    }
}

pub(crate) fn serialize_u128_option<S: serde::Serializer>(
    val: &Option<u128>,
    s: S,
) -> Result<S::Ok, S::Error> {
    match val {
        Some(v) => format!("{}", *v).serialize(s),
        None => s.serialize_unit(),
    }
}
