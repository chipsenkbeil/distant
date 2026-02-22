use std::convert::TryFrom;
use std::io;

use p256::ecdh::EphemeralSecret;
use p256::PublicKey;
use rand::rngs::OsRng;
use sha2::Sha256;

use crate::net::common::SecretKey32;

mod pkb;
pub use pkb::PublicKeyBytes;

mod salt;
pub use salt::Salt;

/// Utility to support performing an exchange of public keys and salts in order to derive a shared
/// key between two separate entities
pub struct KeyExchange {
    secret: EphemeralSecret,
    salt: Salt,
}

impl Default for KeyExchange {
    // Create a new handshake instance with a secret and salt
    fn default() -> Self {
        let secret = EphemeralSecret::random(&mut OsRng);
        let salt = Salt::random();

        Self { secret, salt }
    }
}

impl KeyExchange {
    // Return encoded bytes of public key
    pub fn pk_bytes(&self) -> PublicKeyBytes {
        PublicKeyBytes::from(self.secret.public_key())
    }

    // Return the salt contained by this handshake
    pub fn salt(&self) -> &Salt {
        &self.salt
    }

    /// Derives a shared secret using another key exchange's public key and salt
    pub fn derive_shared_secret(
        &self,
        public_key: PublicKeyBytes,
        salt: Salt,
    ) -> io::Result<SecretKey32> {
        // Decode the public key of the other side
        let decoded_public_key = PublicKey::try_from(public_key)?;

        // Produce a salt that is consistent with what the other side will do
        let shared_salt = self.salt ^ salt;

        // Acquire the shared secret
        let shared_secret = self.secret.diffie_hellman(&decoded_public_key);

        // Extract entropy from the shared secret for use in producing a key
        let hkdf = shared_secret.extract::<Sha256>(Some(shared_salt.as_ref()));

        // Derive a shared key (32 bytes)
        let mut shared_key = [0u8; 32];
        match hkdf.expand(&[], &mut shared_key) {
            Ok(_) => Ok(SecretKey32::from(shared_key)),
            Err(x) => Err(io::Error::new(io::ErrorKind::InvalidData, x.to_string())),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::convert::TryFrom;

    use test_log::test;

    use super::*;

    #[test]
    fn default_should_create_instance_with_unique_keys_and_salts() {
        let a = KeyExchange::default();
        let b = KeyExchange::default();

        // Two independently created instances should have different public keys
        assert_ne!(a.pk_bytes(), b.pk_bytes());
        // Two independently created instances should have different salts
        assert_ne!(a.salt(), b.salt());
    }

    #[test]
    fn pk_bytes_should_return_valid_public_key_bytes() {
        let kx = KeyExchange::default();
        let pk_bytes = kx.pk_bytes();

        // The returned PublicKeyBytes should be convertible back into a valid PublicKey
        let result = PublicKey::try_from(pk_bytes);
        assert!(result.is_ok(), "pk_bytes should produce a valid PublicKey");
    }

    #[test]
    fn pk_bytes_should_return_same_value_on_repeated_calls() {
        let kx = KeyExchange::default();
        let pk1 = kx.pk_bytes();
        let pk2 = kx.pk_bytes();
        assert_eq!(pk1, pk2);
    }

    #[test]
    fn salt_should_return_reference_to_internal_salt() {
        let kx = KeyExchange::default();
        let s1 = kx.salt();
        let s2 = kx.salt();
        assert_eq!(s1, s2);
    }

    #[test]
    fn derive_shared_secret_should_produce_same_key_for_both_sides() {
        let a = KeyExchange::default();
        let b = KeyExchange::default();

        let secret_a = a
            .derive_shared_secret(b.pk_bytes(), *b.salt())
            .expect("a should derive shared secret");
        let secret_b = b
            .derive_shared_secret(a.pk_bytes(), *a.salt())
            .expect("b should derive shared secret");

        assert_eq!(secret_a, secret_b, "Both sides should derive the same key");
    }

    #[test]
    fn derive_shared_secret_should_produce_32_byte_key() {
        let a = KeyExchange::default();
        let b = KeyExchange::default();

        let secret = a
            .derive_shared_secret(b.pk_bytes(), *b.salt())
            .expect("should derive shared secret");

        assert_eq!(secret.len(), 32);
    }

    #[test]
    fn derive_shared_secret_should_differ_for_different_exchanges() {
        let a1 = KeyExchange::default();
        let b1 = KeyExchange::default();
        let a2 = KeyExchange::default();
        let b2 = KeyExchange::default();

        let secret1 = a1
            .derive_shared_secret(b1.pk_bytes(), *b1.salt())
            .expect("first exchange should succeed");
        let secret2 = a2
            .derive_shared_secret(b2.pk_bytes(), *b2.salt())
            .expect("second exchange should succeed");

        assert_ne!(
            secret1, secret2,
            "Different key exchanges should produce different shared secrets"
        );
    }

    #[test]
    fn derive_shared_secret_should_fail_with_invalid_public_key() {
        let kx = KeyExchange::default();

        // Construct a PublicKeyBytes from raw bytes that form a valid EncodedPoint
        // structurally (uncompressed point: 0x04 prefix + 64 bytes) but represent
        // an invalid P-256 curve point.
        let mut invalid_bytes = vec![0x04]; // uncompressed point prefix
        invalid_bytes.extend_from_slice(&[0xFFu8; 64]); // invalid coordinates

        let invalid_pk = PublicKeyBytes::try_from(invalid_bytes)
            .expect("should parse as EncodedPoint structurally");

        let salt = Salt::random();
        let result = kx.derive_shared_secret(invalid_pk, salt);
        assert!(
            result.is_err(),
            "derive_shared_secret should fail with an invalid public key"
        );
    }

    #[test]
    fn derive_shared_secret_should_differ_when_salts_are_swapped() {
        // If a provides its own salt instead of b's salt, the derived key should differ
        let a = KeyExchange::default();
        let b = KeyExchange::default();

        let secret_correct = a
            .derive_shared_secret(b.pk_bytes(), *b.salt())
            .expect("should derive with correct salt");

        // Use a's own salt instead of b's — simulates a protocol mismatch
        let secret_wrong_salt = a
            .derive_shared_secret(b.pk_bytes(), *a.salt())
            .expect("should derive with wrong salt");

        assert_ne!(
            secret_correct, secret_wrong_salt,
            "Using different salts should produce different shared secrets"
        );
    }

    #[test]
    fn derive_shared_secret_should_be_deterministic_for_same_inputs() {
        // Since EphemeralSecret is consumed conceptually after DH, we verify
        // that calling derive_shared_secret twice with the same peer data
        // produces the same result (the secret is borrowed, not consumed).
        let a = KeyExchange::default();
        let b = KeyExchange::default();

        let secret1 = a
            .derive_shared_secret(b.pk_bytes(), *b.salt())
            .expect("first derivation should succeed");
        let secret2 = a
            .derive_shared_secret(b.pk_bytes(), *b.salt())
            .expect("second derivation should succeed");

        assert_eq!(
            secret1, secret2,
            "Same inputs should produce the same shared secret"
        );
    }

    #[test]
    fn derive_shared_secret_with_wrong_peer_key_should_produce_different_secret() {
        let a = KeyExchange::default();
        let b = KeyExchange::default();
        let c = KeyExchange::default();

        // a exchanges with b
        let secret_ab = a
            .derive_shared_secret(b.pk_bytes(), *b.salt())
            .expect("a-b exchange should succeed");

        // a exchanges with c using same salt as b (isolate the key difference)
        let secret_ac = a
            .derive_shared_secret(c.pk_bytes(), *b.salt())
            .expect("a-c exchange should succeed");

        assert_ne!(
            secret_ab, secret_ac,
            "Different peer public keys should produce different shared secrets"
        );
    }
}
