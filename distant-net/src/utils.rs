use serde::{de::DeserializeOwned, Serialize};
use std::io;

pub fn serialize_to_vec<T: Serialize>(value: &T) -> io::Result<Vec<u8>> {
    rmp_serde::encode::to_vec_named(value).map_err(|x| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Serialize failed: {}", x),
        )
    })
}

pub fn deserialize_from_slice<T: DeserializeOwned>(slice: &[u8]) -> io::Result<T> {
    rmp_serde::decode::from_slice(slice).map_err(|x| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Deserialize failed: {}", x),
        )
    })
}
