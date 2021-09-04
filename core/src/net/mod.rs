mod listener;
mod transport;

pub use listener::{AcceptFuture, Listener, TransportListener, TransportListenerCtx};
pub use transport::*;

// Re-export commonly-used orion structs
pub use orion::aead::SecretKey;

pub trait UnprotectedToHexKey {
    fn unprotected_to_hex_key(&self) -> String;
}

impl UnprotectedToHexKey for SecretKey {
    fn unprotected_to_hex_key(&self) -> String {
        hex::encode(self.unprotected_as_bytes())
    }
}
