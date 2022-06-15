use crate::{
    utils, Auth, AuthErrorKind, AuthRequest, AuthResponse, AuthVerifyKind, Client, Codec,
    Handshake, Question, XChaCha20Poly1305Codec,
};
use bytes::BytesMut;
use std::{collections::HashMap, io};

pub struct AuthClient {
    inner: Client<Auth, Auth>,
    codec: Option<XChaCha20Poly1305Codec>,
}

impl From<Client<Auth, Auth>> for AuthClient {
    fn from(client: Client<Auth, Auth>) -> Self {
        Self {
            inner: client,
            codec: None,
        }
    }
}

impl AuthClient {
    /// Sends a request to the server to establish an encrypted connection
    pub async fn handshake(&mut self) -> io::Result<()> {
        let handshake = Handshake::default();

        let response = self
            .inner
            .send(Auth::Handshake {
                public_key: handshake.pk_bytes(),
                salt: *handshake.salt(),
            })
            .await?;

        match response.payload {
            Auth::Handshake { public_key, salt } => {
                let key = handshake.handshake(public_key, salt)?;
                self.codec.replace(XChaCha20Poly1305Codec::new(&key));
                Ok(())
            }
            Auth::Msg { .. } => Err(io::Error::new(
                io::ErrorKind::Other,
                "Got unexpected encrypted message during handshake",
            )),
        }
    }

    /// Returns true if client has successfully performed a handshake
    /// and is ready to communicate with the server
    pub fn is_ready(&self) -> bool {
        self.codec.is_some()
    }

    /// Provides a challenge to the server and returns the answers to the questions
    /// asked by the client
    pub async fn challenge(
        &mut self,
        questions: Vec<Question>,
        extra: HashMap<String, String>,
    ) -> io::Result<Vec<String>> {
        let payload = AuthRequest::Challenge { questions, extra };
        let encrypted_payload = self.serialize_and_encrypt(&payload)?;
        let response = self.inner.send(Auth::Msg { encrypted_payload }).await?;

        match response.payload {
            Auth::Msg { encrypted_payload } => {
                match self.decrypt_and_deserialize(&encrypted_payload)? {
                    AuthResponse::Challenge { answers } => Ok(answers),
                    AuthResponse::Verify { .. } => Err(io::Error::new(
                        io::ErrorKind::Other,
                        "Got unexpected verify response during challenge",
                    )),
                }
            }
            Auth::Handshake { .. } => Err(io::Error::new(
                io::ErrorKind::Other,
                "Got unexpected handshake during challenge",
            )),
        }
    }

    /// Provides a verification request to the server and returns whether or not
    /// the server approved
    pub async fn verify(&mut self, kind: AuthVerifyKind, text: String) -> io::Result<bool> {
        let payload = AuthRequest::Verify { kind, text };
        let encrypted_payload = self.serialize_and_encrypt(&payload)?;
        let response = self.inner.send(Auth::Msg { encrypted_payload }).await?;

        match response.payload {
            Auth::Msg { encrypted_payload } => {
                match self.decrypt_and_deserialize(&encrypted_payload)? {
                    AuthResponse::Verify { valid } => Ok(valid),
                    AuthResponse::Challenge { .. } => Err(io::Error::new(
                        io::ErrorKind::Other,
                        "Got unexpected challenge response during verify",
                    )),
                }
            }
            Auth::Handshake { .. } => Err(io::Error::new(
                io::ErrorKind::Other,
                "Got unexpected handshake during verify",
            )),
        }
    }

    /// Provides information to the server to use as it pleases with no response expected
    pub async fn info(&mut self, text: String) -> io::Result<()> {
        let payload = AuthRequest::Info { text };
        let encrypted_payload = self.serialize_and_encrypt(&payload)?;
        self.inner.fire(Auth::Msg { encrypted_payload }).await
    }

    /// Provides an error to the server to use as it pleases with no response expected
    pub async fn error(&mut self, kind: AuthErrorKind, text: String) -> io::Result<()> {
        let payload = AuthRequest::Error { kind, text };
        let encrypted_payload = self.serialize_and_encrypt(&payload)?;
        self.inner.fire(Auth::Msg { encrypted_payload }).await
    }

    fn serialize_and_encrypt(&mut self, payload: &AuthRequest) -> io::Result<Vec<u8>> {
        let codec = self.codec.as_mut().ok_or_else(|| {
            io::Error::new(io::ErrorKind::Other, "Handshake must be performed first")
        })?;

        let mut encryped_payload = BytesMut::new();
        let payload = utils::serialize_to_vec(payload)?;
        codec.encode(&payload, &mut encryped_payload)?;
        Ok(encryped_payload.freeze().to_vec())
    }

    fn decrypt_and_deserialize(&mut self, payload: &[u8]) -> io::Result<AuthResponse> {
        let codec = self.codec.as_mut().ok_or_else(|| {
            io::Error::new(io::ErrorKind::Other, "Handshake must be performed first")
        })?;

        let mut payload = BytesMut::from(payload);
        match codec.decode(&mut payload)? {
            Some(payload) => utils::deserialize_from_slice::<AuthResponse>(&payload),
            None => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Incomplete message received",
            )),
        }
    }
}
