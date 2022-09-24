use super::{msg::*, AuthHandler};
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
        questions: Vec<Question>,
        options: HashMap<String, String>,
    ) -> io::Result<Vec<String>>;

    /// Requests verification of some `kind` and `text`, returning true if passed verification.
    async fn verify(&mut self, kind: VerificationKind, text: String) -> io::Result<bool>;

    /// Reports information with no response expected.
    async fn info(&mut self, text: String) -> io::Result<()>;

    /// Reports an error occurred during authentication, consuming the authenticator since no more
    /// challenges should be issued.
    async fn error(&mut self, kind: ErrorKind, text: String) -> io::Result<()>;

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
impl<T> Authenticate for FramedTransport<T>
where
    T: Transport + Send + Sync,
{
    async fn authenticate(&mut self, mut handler: impl AuthHandler + Send) -> io::Result<()> {
        loop {
            match next_frame_as!(self, Authentication) {
                Authentication::Challenge(x) => {
                    trace!("Authenticate::Challenge({x:?})");
                    let answers = handler.on_challenge(x.questions, x.options).await?;
                    write_frame!(
                        self,
                        AuthenticationResponse::Challenge(ChallengeResponse { answers })
                    );
                }
                Authentication::Verification(x) => {
                    trace!("Authenticate::Verify({x:?})");
                    let valid = handler.on_verify(x.kind, x.text).await?;
                    write_frame!(
                        self,
                        AuthenticationResponse::Verification(VerificationResponse { valid })
                    );
                }
                Authentication::Info(x) => {
                    trace!("Authenticate::Info({x:?})");
                    handler.on_info(x.text).await?;
                }
                Authentication::Error(x) => {
                    trace!("Authenticate::Error({x:?})");
                    let kind = x.kind;
                    let text = x.text;

                    handler.on_error(kind, &text).await?;

                    return Err(match kind {
                        ErrorKind::FailedChallenge => io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            format!("Failed challenge: {text}"),
                        ),
                        ErrorKind::FailedVerification => io::Error::new(
                            io::ErrorKind::PermissionDenied,
                            format!("Failed verification: {text}"),
                        ),
                        ErrorKind::Unknown => {
                            io::Error::new(io::ErrorKind::Other, format!("Unknown error: {text}"))
                        }
                    });
                }
                Authentication::Start(x) => {
                    trace!("Authenticate::Start({x:?})");
                    return Ok(());
                }
                Authentication::Finished => {
                    trace!("Authenticate::Finished");
                    return Ok(());
                }
            }
        }
    }
}

#[async_trait]
impl<T> Authenticator for FramedTransport<T>
where
    T: Transport + Send + Sync,
{
    async fn challenge(
        &mut self,
        questions: Vec<Question>,
        options: HashMap<String, String>,
    ) -> io::Result<Vec<String>> {
        trace!("Authenticator::challenge(questions = {questions:?}, options = {options:?})");

        write_frame!(self, Authentication::from(Challenge { questions, options }));
        let response = next_frame_as!(self, AuthenticationResponse, Challenge);
        Ok(response.answers)
    }

    async fn verify(&mut self, kind: VerificationKind, text: String) -> io::Result<bool> {
        trace!("Authenticator::verify(kind = {kind:?}, text = {text:?})");

        write_frame!(self, Authentication::from(Verification { kind, text }));
        let response = next_frame_as!(self, AuthenticationResponse, Verification);
        Ok(response.valid)
    }

    async fn info(&mut self, text: String) -> io::Result<()> {
        trace!("Authenticator::info(text = {text:?})");
        write_frame!(self, Authentication::from(Info { text }));
        Ok(())
    }

    async fn error(&mut self, kind: ErrorKind, text: String) -> io::Result<()> {
        trace!("Authenticator::error(kind = {kind:?}, text = {text:?})");
        write_frame!(self, Authentication::from(Error { kind, text }));
        Ok(())
    }

    async fn finished(&mut self) -> io::Result<()> {
        trace!("Authenticator::finished()");
        write_frame!(self, Authentication::Finished);
        Ok(())
    }
}
