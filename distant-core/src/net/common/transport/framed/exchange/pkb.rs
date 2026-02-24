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

#[cfg(test)]
mod tests {
    //! Tests for PublicKeyBytes: round-trip conversions (PublicKey <-> PKB <-> Vec), error
    //! handling for invalid/garbage/wrong-length bytes, serde JSON round-trips, uniqueness,
    //! clone, and debug formatting.

    use super::*;
    use p256::ecdh::EphemeralSecret;
    use rand::rngs::OsRng;
    use test_log::test;

    /// Helper: generate a fresh PublicKey from an ephemeral secret
    fn generate_public_key() -> PublicKey {
        let secret = EphemeralSecret::random(&mut OsRng);
        secret.public_key()
    }

    #[test]
    fn public_key_to_pkb_to_public_key_round_trip() {
        let pk = generate_public_key();
        let pkb = PublicKeyBytes::from(pk);
        let recovered = PublicKey::try_from(pkb).expect("Should convert back to PublicKey");
        assert_eq!(pk, recovered);
    }

    #[test]
    fn pkb_to_vec_to_pkb_round_trip() {
        let pk = generate_public_key();
        let pkb = PublicKeyBytes::from(pk);
        let bytes: Vec<u8> = pkb.clone().into();
        let recovered =
            PublicKeyBytes::try_from(bytes).expect("Should convert back to PublicKeyBytes");
        assert_eq!(pkb, recovered);
    }

    #[test]
    fn pkb_to_vec_should_produce_non_empty_bytes() {
        let pk = generate_public_key();
        let pkb = PublicKeyBytes::from(pk);
        let bytes: Vec<u8> = pkb.into();
        assert!(!bytes.is_empty());
    }

    #[test]
    fn try_from_vec_should_fail_with_empty_bytes() {
        let result = PublicKeyBytes::try_from(Vec::new());
        assert!(result.is_err());
    }

    #[test]
    fn try_from_vec_should_fail_with_random_garbage() {
        let garbage = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03];
        let result = PublicKeyBytes::try_from(garbage);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn try_from_vec_should_fail_with_wrong_length() {
        // A valid SEC1 uncompressed point for p256 is 65 bytes (0x04 prefix + 64).
        // Provide 64 bytes with the right prefix but truncated.
        let mut bad_bytes = vec![0x04];
        bad_bytes.extend_from_slice(&[0xAA; 63]); // 64 total, should be 65
        let result = PublicKeyBytes::try_from(bad_bytes);
        assert!(result.is_err());
    }

    #[test]
    fn serde_json_round_trip() {
        let pk = generate_public_key();
        let pkb = PublicKeyBytes::from(pk);
        let json = serde_json::to_string(&pkb).expect("Serialize failed");
        let deserialized: PublicKeyBytes = serde_json::from_str(&json).expect("Deserialize failed");
        assert_eq!(pkb, deserialized);
    }

    #[test]
    fn serde_json_preserves_public_key_value() {
        let pk = generate_public_key();
        let pkb = PublicKeyBytes::from(pk);
        let json = serde_json::to_string(&pkb).expect("Serialize failed");
        let deserialized: PublicKeyBytes = serde_json::from_str(&json).expect("Deserialize failed");
        let recovered_pk =
            PublicKey::try_from(deserialized).expect("Should convert back to PublicKey");
        assert_eq!(pk, recovered_pk);
    }

    #[test]
    fn serde_json_should_fail_with_invalid_bytes() {
        let garbage: Vec<u8> = vec![0xFF; 10];
        let json = serde_json::to_string(&garbage).unwrap();
        let result: Result<PublicKeyBytes, _> = serde_json::from_str(&json);
        assert!(result.is_err());
    }

    #[test]
    fn different_keys_produce_different_pkb() {
        let pk1 = generate_public_key();
        let pk2 = generate_public_key();
        let pkb1 = PublicKeyBytes::from(pk1);
        let pkb2 = PublicKeyBytes::from(pk2);
        assert_ne!(pkb1, pkb2);
    }

    #[test]
    fn clone_should_produce_equal_pkb() {
        let pk = generate_public_key();
        let pkb = PublicKeyBytes::from(pk);
        let cloned = pkb.clone();
        assert_eq!(pkb, cloned);
    }

    #[test]
    fn debug_should_not_panic() {
        let pk = generate_public_key();
        let pkb = PublicKeyBytes::from(pk);
        let debug_str = format!("{:?}", pkb);
        assert!(!debug_str.is_empty());
    }
}
