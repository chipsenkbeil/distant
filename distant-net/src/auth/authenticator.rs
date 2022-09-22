use super::{data::*, AuthHandler};
use crate::{utils, FramedTransport, Transport};
use async_trait::async_trait;
use log::*;
use std::{collections::HashMap, io};

/// Represents an interface for authenticating with a server.
#[async_trait]
pub trait Authenticate {
    /// Performs authentication by leveraging the `handler` for any received challenge.
    async fn authenticate(&mut self, mut handler: impl AuthHandler + Send) -> io::Result<()>;
}

/// Represents an interface for submitting challenges for authentication.
#[async_trait]
pub trait Authenticator: Send {
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
    async fn error(&mut self, kind: AuthErrorKind, text: String) -> io::Result<()>;

    /// Reports that the authentication has finished successfully, consuming the authenticator
    /// since no more challenges should be issued.
    async fn finished(&mut self) -> io::Result<()>;
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
impl<T, const CAPACITY: usize> Authenticate for FramedTransport<T, CAPACITY>
where
    T: Transport + Send + Sync,
{
    async fn authenticate(&mut self, mut handler: impl AuthHandler + Send) -> io::Result<()> {
        loop {
            match next_frame_as!(self, AuthRequest) {
                AuthRequest::Challenge(x) => {
                    trace!("Authenticate::Challenge({x:?})");
                    let answers = handler.on_challenge(x.questions, x.options).await?;
                    write_frame!(
                        self,
                        AuthResponse::Challenge(AuthChallengeResponse { answers })
                    );
                }
                AuthRequest::Verify(x) => {
                    trace!("Authenticate::Verify({x:?})");
                    let valid = handler.on_verify(x.kind, x.text).await?;
                    write_frame!(self, AuthResponse::Verify(AuthVerifyResponse { valid }));
                }
                AuthRequest::Info(x) => {
                    trace!("Authenticate::Info({x:?})");
                    handler.on_info(x.text).await?;
                }
                AuthRequest::Error(x) => {
                    trace!("Authenticate::Error({x:?})");
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
                AuthRequest::Finished => {
                    trace!("Authenticate::Finished");
                    return Ok(());
                }
            }
        }
    }
}

#[async_trait]
impl<T, const CAPACITY: usize> Authenticator for FramedTransport<T, CAPACITY>
where
    T: Transport + Send + Sync,
{
    async fn challenge(
        &mut self,
        questions: Vec<AuthQuestion>,
        options: HashMap<String, String>,
    ) -> io::Result<Vec<String>> {
        trace!("Authenticator::challenge(questions = {questions:?}, options = {options:?})");

        write_frame!(
            self,
            AuthRequest::from(AuthChallengeRequest { questions, options })
        );
        let response = next_frame_as!(self, AuthResponse, Challenge);
        Ok(response.answers)
    }

    async fn verify(&mut self, kind: AuthVerifyKind, text: String) -> io::Result<bool> {
        trace!("Authenticator::verify(kind = {kind:?}, text = {text:?})");

        write_frame!(self, AuthRequest::from(AuthVerifyRequest { kind, text }));
        let response = next_frame_as!(self, AuthResponse, Verify);
        Ok(response.valid)
    }

    async fn info(&mut self, text: String) -> io::Result<()> {
        trace!("Authenticator::info(text = {text:?})");
        write_frame!(self, AuthRequest::from(AuthInfo { text }));
        Ok(())
    }

    async fn error(&mut self, kind: AuthErrorKind, text: String) -> io::Result<()> {
        trace!("Authenticator::error(kind = {kind:?}, text = {text:?})");
        write_frame!(self, AuthRequest::from(AuthError { kind, text }));
        Ok(())
    }

    async fn finished(&mut self) -> io::Result<()> {
        trace!("Authenticator::finished()");
        write_frame!(self, AuthRequest::Finished);
        Ok(())
    }
}
