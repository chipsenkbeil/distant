use super::{data::*, AuthHandler};
use crate::{utils, FramedTransport, Transport};
use async_trait::async_trait;
use log::*;
use std::{collections::HashMap, io};

/// Represents an interface for authenticating or submitting challenges for authentication.
#[async_trait]
pub trait Authenticator: Send {
    /// Performs authentication by leveraging the `handler` for any received challenge.
    async fn authenticate(&mut self, mut handler: impl AuthHandler + Send) -> io::Result<()>;

    /// Issues a challenge and returns the answers to the `questions` asked.
    async fn challenge(
        &mut self,
        questions: Vec<AuthQuestion>,
        options: HashMap<String, String>,
    ) -> io::Result<Vec<String>>;

    /// Requests verification of some `kind` and `text`, returning true if passed verification.
    async fn verify(&mut self, kind: AuthVerifyKind, text: String) -> io::Result<bool>;

    /// Reports information with no response expected.
    async fn info(&mut self, text: String) -> io::Result<()>;

    /// Reports an error occurred during authentication, consuming the authenticator since no more
    /// challenges should be issued.
    async fn error(self, kind: AuthErrorKind, text: String) -> io::Result<()>;

    /// Reports that the authentication has finished successfully, consuming the authenticator
    /// since no more challenges should be issued.
    async fn finished(self) -> io::Result<()>;
}

/// Wraps a [`FramedTransport`] in order to perform challenge-based communication through the
/// transport to authenticate it. The authenticator is capable of conducting challenges or
/// leveraging an [`AuthHandler`] to process challenges.
pub struct FramedAuthenticator<'a, T: Send, const CAPACITY: usize> {
    transport: &'a mut FramedTransport<T, CAPACITY>,
}

impl<'a, T: Send, const CAPACITY: usize> FramedAuthenticator<'a, T, CAPACITY> {
    pub fn new(transport: &'a mut FramedTransport<T, CAPACITY>) -> Self {
        Self { transport }
    }
}

macro_rules! write_frame {
    ($transport:expr, $data:expr) => {{
        $transport
            .write_frame(utils::serialize_to_vec(&$data)?)
            .await?
    }};
}

macro_rules! next_frame_as {
    ($transport:expr, $type:ident, $variant:ident) => {{
        match { next_frame_as!($transport, $type) } {
            $type::$variant(x) => x,
            x => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Unexpected frame: {x:?}"),
                ))
            }
        }
    }};
    ($transport:expr, $type:ident) => {{
        let frame = $transport.read_frame().await?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::UnexpectedEof, "Transport closed early")
        })?;

        utils::deserialize_from_slice::<$type>(frame.as_item())?
    }};
}

#[async_trait]
impl<'a, T, const CAPACITY: usize> Authenticator for FramedAuthenticator<'a, T, CAPACITY>
where
    T: Transport + Send + Sync,
{
    /// Performs authentication by leveraging the `handler` for any received challenge.
    async fn authenticate(&mut self, mut handler: impl AuthHandler + Send) -> io::Result<()> {
        loop {
            match next_frame_as!(self.transport, AuthRequest) {
                AuthRequest::Challenge(x) => {
                    let answers = handler.on_challenge(x.questions, x.options).await?;
                    write_frame!(
                        self.transport,
                        AuthResponse::Challenge(AuthChallengeResponse { answers })
                    );
                }
                AuthRequest::Verify(x) => {
                    let valid = handler.on_verify(x.kind, x.text).await?;
                    write_frame!(
                        self.transport,
                        AuthResponse::Verify(AuthVerifyResponse { valid })
                    );
                }
                AuthRequest::Info(x) => {
                    handler.on_info(x.text).await?;
                }
                AuthRequest::Error(x) => {
                    let kind = x.kind;
                    let text = x.text;

                    handler.on_error(kind, &text).await?;

                    return Err(match kind {
                        AuthErrorKind::FailedChallenge => io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            format!("Failed challenge: {text}"),
                        ),
                        AuthErrorKind::FailedVerification => io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            format!("Failed verification: {text}"),
                        ),
                        AuthErrorKind::Unknown => {
                            io::Error::new(io::ErrorKind::Other, format!("Unknown error: {text}"))
                        }
                    });
                }
                AuthRequest::Finished => return Ok(()),
            }
        }
    }

    /// Issues a challenge and returns the answers to the `questions` asked.
    async fn challenge(
        &mut self,
        questions: Vec<AuthQuestion>,
        options: HashMap<String, String>,
    ) -> io::Result<Vec<String>> {
        trace!(
            "Authenticator::challenge(questions = {:?}, options = {:?})",
            questions,
            options
        );

        write_frame!(
            self.transport,
            AuthRequest::from(AuthChallengeRequest { questions, options })
        );
        let response = next_frame_as!(self.transport, AuthResponse, Challenge);
        Ok(response.answers)
    }

    /// Requests verification of some `kind` and `text`, returning true if passed verification.
    async fn verify(&mut self, kind: AuthVerifyKind, text: String) -> io::Result<bool> {
        trace!(
            "Authenticator::verify(kind = {:?}, text = {:?})",
            kind,
            text
        );

        write_frame!(
            self.transport,
            AuthRequest::from(AuthVerifyRequest { kind, text })
        );
        let response = next_frame_as!(self.transport, AuthResponse, Verify);
        Ok(response.valid)
    }

    /// Reports information with no response expected.
    async fn info(&mut self, text: String) -> io::Result<()> {
        trace!("Authenticator::info(text = {:?})", text);
        write_frame!(self.transport, AuthRequest::from(AuthInfo { text }));
        Ok(())
    }

    /// Reports an error occurred during authentication, consuming the authenticator since no more
    /// challenges should be issued.
    async fn error(self, kind: AuthErrorKind, text: String) -> io::Result<()> {
        trace!("Authenticator::error(kind = {:?}, text = {:?})", kind, text);
        write_frame!(self.transport, AuthRequest::from(AuthError { kind, text }));
        Ok(())
    }

    /// Reports that the authentication has finished successfully, consuming the authenticator
    /// since no more challenges should be issued.
    async fn finished(self) -> io::Result<()> {
        trace!("Authenticator::finished()");
        write_frame!(self.transport, AuthRequest::Finished);
        Ok(())
    }
}
