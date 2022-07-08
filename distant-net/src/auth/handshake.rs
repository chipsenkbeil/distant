use p256::{ecdh::EphemeralSecret, EncodedPoint, PublicKey};
use rand::rngs::OsRng;
use sha2::Sha256;
use std::io;

mod salt;
pub use salt::Salt;

/// 32-byte key shared by handshake
pub type SharedKey = [u8; 32];

/// Utility to perform a handshake
pub struct Handshake {
    secret: EphemeralSecret,
    salt: Salt,
}

impl Default for Handshake {
    // Create a new handshake instance with a secret and salt
    fn default() -> Self {
        let secret = EphemeralSecret::random(&mut OsRng);
        let salt = Salt::random();

        Self { secret, salt }
    }
}

impl Handshake {
    // Return encoded bytes of public key
    pub fn pk_bytes(&self) -> EncodedPoint {
        EncodedPoint::from(self.secret.public_key())
    }

    // Return the salt contained by this handshake
    pub fn salt(&self) -> &Salt {
        &self.salt
    }

    pub fn handshake(&self, public_key: EncodedPoint, salt: Salt) -> io::Result<SharedKey> {
        // Decode the public key of the client
        let decoded_public_key = match PublicKey::from_sec1_bytes(public_key.as_ref()) {
            Ok(x) => x,
            Err(x) => {
                return Err(io::Error::new(io::ErrorKind::InvalidData, x.to_string()));
            }
        };

        // Produce a salt that is consistent with what the other side will do
        let shared_salt = self.salt ^ salt;

        // Acquire the shared secret
        let shared_secret = self.secret.diffie_hellman(&decoded_public_key);

        // Extract entropy from the shared secret for use in producing a key
        let hkdf = shared_secret.extract::<Sha256>(Some(shared_salt.as_slice()));

        // Derive a shared key (32 bytes)
        let mut shared_key = [0u8; 32];
        match hkdf.expand(&[], &mut shared_key) {
            Ok(_) => Ok(shared_key),
            Err(x) => Err(io::Error::new(io::ErrorKind::InvalidData, x.to_string())),
        }
    }
}
