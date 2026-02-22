use std::convert::TryFrom;
use std::io;

use p256::{EncodedPoint, PublicKey};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Represents a wrapper around [`EncodedPoint`], and exists to
/// fix an issue with [`serde`] deserialization failing when
/// directly serializing the [`EncodedPoint`] type
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "Vec<u8>", try_from = "Vec<u8>")]
pub struct PublicKeyBytes(EncodedPoint);

impl From<PublicKey> for PublicKeyBytes {
    fn from(pk: PublicKey) -> Self {
        Self(EncodedPoint::from(pk))
    }
}

impl TryFrom<PublicKeyBytes> for PublicKey {
    type Error = io::Error;

    fn try_from(pkb: PublicKeyBytes) -> Result<Self, Self::Error> {
        PublicKey::from_sec1_bytes(pkb.0.as_ref())
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))
    }
}

impl From<PublicKeyBytes> for Vec<u8> {
    fn from(pkb: PublicKeyBytes) -> Self {
        pkb.0.as_bytes().to_vec()
    }
}

impl TryFrom<Vec<u8>> for PublicKeyBytes {
    type Error = io::Error;

    fn try_from(bytes: Vec<u8>) -> Result<Self, Self::Error> {
        Ok(Self(EncodedPoint::from_bytes(bytes).map_err(|x| {
            io::Error::new(io::ErrorKind::InvalidData, x.to_string())
        })?))
    }
}

impl serde_bytes::Serialize for PublicKeyBytes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(self.0.as_ref())
    }
}

impl<'de> serde_bytes::Deserialize<'de> for PublicKeyBytes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = Deserialize::deserialize(deserializer).map(serde_bytes::ByteBuf::into_vec)?;
        PublicKeyBytes::try_from(bytes).map_err(serde::de::Error::custom)
    }
}
