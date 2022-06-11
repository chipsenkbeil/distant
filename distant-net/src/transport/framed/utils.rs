use serde::{de::DeserializeOwned, Serialize};
use std::io;

pub fn serialize_to_vec<T: Serialize>(value: &T) -> io::Result<Vec<u8>> {
    let mut v = Vec::new();

    let _ = ciborium::ser::into_writer(value, &mut v).map_err(|x| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Serialize failed: {}", x),
        )
    })?;

    Ok(v)
}

pub fn deserialize_from_slice<T: DeserializeOwned>(slice: &[u8]) -> io::Result<T> {
    ciborium::de::from_reader(slice).map_err(|x| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Deserialize failed: {}", x),
        )
    })
}
