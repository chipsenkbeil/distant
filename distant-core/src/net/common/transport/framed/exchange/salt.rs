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

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    #[test]
    fn random_should_produce_32_byte_salt() {
        let salt = Salt::random();
        let bytes: Vec<u8> = salt.into();
        assert_eq!(bytes.len(), 32);
    }

    #[test]
    fn two_random_salts_should_differ() {
        let a = Salt::random();
        let b = Salt::random();
        assert_ne!(a, b);
    }

    #[test]
    fn display_should_produce_64_char_hex_string() {
        let salt = Salt::random();
        let s = salt.to_string();
        assert_eq!(s.len(), 64);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn display_and_from_str_should_round_trip() {
        let original = Salt::random();
        let hex_str = original.to_string();
        let parsed: Salt = hex_str.parse().expect("Failed to parse hex string");
        assert_eq!(original, parsed);
    }

    #[test]
    fn from_str_should_fail_on_invalid_hex() {
        let result = Salt::from_str("zzzz_not_hex");
        assert!(result.is_err());
    }

    #[test]
    fn from_str_should_fail_on_wrong_length_hex() {
        // 30 bytes encoded as 60 hex chars — valid hex but wrong length
        let short_hex = "ab".repeat(30);
        let result = Salt::from_str(&short_hex);
        assert!(result.is_err());
    }

    #[test]
    fn try_from_vec_u8_should_succeed_with_32_bytes() {
        let bytes = vec![0xABu8; 32];
        let salt = Salt::try_from(bytes.clone()).expect("Should succeed with 32 bytes");
        assert_eq!(salt.as_ref(), &bytes[..]);
    }

    #[test]
    fn try_from_vec_u8_should_fail_with_too_few_bytes() {
        let bytes = vec![0u8; 16];
        let result = Salt::try_from(bytes);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("16"));
    }

    #[test]
    fn try_from_vec_u8_should_fail_with_too_many_bytes() {
        let bytes = vec![0u8; 64];
        let result = Salt::try_from(bytes);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("64"));
    }

    #[test]
    fn try_from_vec_u8_should_fail_with_empty_vec() {
        let result = Salt::try_from(Vec::new());
        assert!(result.is_err());
    }

    #[test]
    fn try_from_string_should_work() {
        let original = Salt::random();
        let hex_string: String = original.into();
        let parsed = Salt::try_from(hex_string).expect("Should parse from String");
        assert_eq!(original, parsed);
    }

    #[test]
    fn try_from_string_should_fail_on_invalid_input() {
        let result = Salt::try_from("not valid hex".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn from_salt_for_string_should_produce_hex() {
        let salt = Salt::random();
        let s: String = salt.into();
        assert_eq!(s.len(), 64);
        assert!(s.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn from_salt_for_vec_u8_should_return_32_bytes() {
        let salt = Salt::random();
        let bytes: Vec<u8> = salt.into();
        assert_eq!(bytes.len(), 32);
    }

    #[test]
    fn from_salt_for_vec_u8_should_preserve_content() {
        let raw = vec![42u8; 32];
        let salt = Salt::try_from(raw.clone()).unwrap();
        let recovered: Vec<u8> = salt.into();
        assert_eq!(recovered, raw);
    }

    #[test]
    fn as_ref_should_return_inner_bytes() {
        let raw = vec![7u8; 32];
        let salt = Salt::try_from(raw.clone()).unwrap();
        assert_eq!(salt.as_ref(), &raw[..]);
    }

    #[test]
    fn bitxor_should_be_commutative() {
        let a = Salt::random();
        let b = Salt::random();
        assert_eq!(a ^ b, b ^ a);
    }

    #[test]
    fn bitxor_self_should_produce_all_zeros() {
        let salt = Salt::random();
        let result = salt ^ salt;
        let bytes: Vec<u8> = result.into();
        assert!(bytes.iter().all(|&b| b == 0));
    }

    #[test]
    fn bitxor_with_zero_should_be_identity() {
        let salt = Salt::random();
        let zero = Salt::try_from(vec![0u8; 32]).unwrap();
        assert_eq!(salt ^ zero, salt);
    }

    #[test]
    fn bitxor_known_values() {
        let a = Salt::try_from(vec![0xFF; 32]).unwrap();
        let b = Salt::try_from(vec![0x00; 32]).unwrap();
        let result = a ^ b;
        let bytes: Vec<u8> = result.into();
        assert!(bytes.iter().all(|&b| b == 0xFF));

        let c = Salt::try_from(vec![0xFF; 32]).unwrap();
        let d = Salt::try_from(vec![0xFF; 32]).unwrap();
        let result2 = c ^ d;
        let bytes2: Vec<u8> = result2.into();
        assert!(bytes2.iter().all(|&b| b == 0x00));
    }

    #[test]
    fn serde_json_round_trip() {
        let original = Salt::random();
        let json = serde_json::to_string(&original).expect("Serialize failed");
        let deserialized: Salt = serde_json::from_str(&json).expect("Deserialize failed");
        assert_eq!(original, deserialized);
    }

    #[test]
    fn serde_json_with_known_bytes() {
        let raw = vec![0x01u8; 32];
        let salt = Salt::try_from(raw.clone()).unwrap();
        let json = serde_json::to_string(&salt).expect("Serialize failed");
        let deserialized: Salt = serde_json::from_str(&json).expect("Deserialize failed");
        let recovered: Vec<u8> = deserialized.into();
        assert_eq!(recovered, raw);
    }

    #[test]
    fn serde_json_should_fail_with_wrong_length() {
        // Manually craft JSON with a 16-byte array
        let short_bytes: Vec<u8> = vec![0u8; 16];
        let json = serde_json::to_string(&short_bytes).unwrap();
        let result: Result<Salt, _> = serde_json::from_str(&json);
        assert!(result.is_err());
    }

    #[test]
    fn clone_should_produce_equal_salt() {
        let salt = Salt::random();
        let cloned = salt;
        assert_eq!(salt, cloned);
    }

    #[test]
    fn debug_should_not_panic() {
        let salt = Salt::random();
        let debug_str = format!("{:?}", salt);
        assert!(!debug_str.is_empty());
    }
}
