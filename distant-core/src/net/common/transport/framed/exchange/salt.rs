use std::convert::{TryFrom, TryInto};
use std::ops::BitXor;
use std::str::FromStr;
use std::{fmt, io};

use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Friendly wrapper around a 32-byte array representing a salt
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "Vec<u8>", try_from = "Vec<u8>")]
pub struct Salt([u8; 32]);

impl Salt {
    /// Generates a salt via a uniform random
    pub fn random() -> Self {
        let mut salt = [0u8; 32];
        OsRng.fill_bytes(&mut salt);
        Self(salt)
    }
}

impl fmt::Display for Salt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

impl serde_bytes::Serialize for Salt {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(self.as_ref())
    }
}

impl<'de> serde_bytes::Deserialize<'de> for Salt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let bytes = Deserialize::deserialize(deserializer).map(serde_bytes::ByteBuf::into_vec)?;
        let bytes_len = bytes.len();
        Salt::try_from(bytes)
            .map_err(|_| serde::de::Error::invalid_length(bytes_len, &"expected 32-byte length"))
    }
}

impl From<Salt> for String {
    fn from(salt: Salt) -> Self {
        salt.to_string()
    }
}

impl FromStr for Salt {
    type Err = io::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bytes = hex::decode(s).map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;
        Self::try_from(bytes)
    }
}

impl TryFrom<String> for Salt {
    type Error = io::Error;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl TryFrom<Vec<u8>> for Salt {
    type Error = io::Error;

    fn try_from(bytes: Vec<u8>) -> Result<Self, Self::Error> {
        Ok(Self(bytes.try_into().map_err(|x: Vec<u8>| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Vec<u8> len of {} != 32", x.len()),
            )
        })?))
    }
}

impl From<Salt> for Vec<u8> {
    fn from(salt: Salt) -> Self {
        salt.0.to_vec()
    }
}

impl AsRef<[u8]> for Salt {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl BitXor for Salt {
    type Output = Self;

    fn bitxor(self, rhs: Self) -> Self::Output {
        let shared_salt = self
            .0
            .iter()
            .zip(rhs.0.iter())
            .map(|(x, y)| x ^ y)
            .collect::<Vec<u8>>();
        Self::try_from(shared_salt).unwrap()
    }
}
