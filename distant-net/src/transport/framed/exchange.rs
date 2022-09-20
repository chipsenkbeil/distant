use super::HeapSecretKey;
use p256::{ecdh::EphemeralSecret, PublicKey};
use rand::rngs::OsRng;
use sha2::Sha256;
use std::{convert::TryFrom, io};

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
    ) -> io::Result<HeapSecretKey> {
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
            Ok(_) => Ok(HeapSecretKey::from(shared_key)),
            Err(x) => Err(io::Error::new(io::ErrorKind::InvalidData, x.to_string())),
        }
    }
}
