use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::{
    convert::{TryFrom, TryInto},
    fmt, io,
    ops::BitXor,
    str::FromStr,
};

/// 32-byte uniform random
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct Salt([u8; 32]);

impl Salt {
    pub fn as_slice(&self) -> &[u8] {
        self.0.as_ref()
    }

    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_vec()
    }

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
