use super::{msg::*, AuthHandler};
use crate::{utils, FramedTransport, Transport};
use async_trait::async_trait;
use log::*;
use std::io;

/// Represents an interface for authenticating with a server.
#[async_trait]
pub trait Authenticate {
    /// Performs authentication by leveraging the `handler` for any received challenge.
    async fn authenticate(&mut self, mut handler: impl AuthHandler + Send) -> io::Result<()>;
}

/// Represents an interface for submitting challenges for authentication.
#[async_trait]
pub trait Authenticator: Send {
    /// Issues an initialization notice and returns the response indicating which authentication
    /// methods to pursue
    async fn initialize(
        &mut self,
        initialization: Initialization,
    ) -> io::Result<InitializationResponse>;

    /// Issues a challenge and returns the answers to the `questions` asked.
    async fn challenge(&mut self, challenge: Challenge) -> io::Result<ChallengeResponse>;

    /// Requests verification of some `kind` and `text`, returning true if passed verification.
    async fn verify(&mut self, verification: Verification) -> io::Result<VerificationResponse>;

    /// Reports information with no response expected.
    async fn info(&mut self, info: Info) -> io::Result<()>;

    /// Reports an error occurred during authentication, consuming the authenticator since no more
    /// challenges should be issued.
    async fn error(&mut self, error: Error) -> io::Result<()>;

    /// Reports that the authentication has started for a specific method.
    async fn start_method(&mut self, start_method: StartMethod) -> io::Result<()>;

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
            trace!("Authenticate::authenticate waiting on next authentication frame");
            match next_frame_as!(self, Authentication) {
                Authentication::Initialization(x) => {
                    trace!("Authenticate::Initialization({x:?})");
                    let response = handler.on_initialization(x).await?;
                    write_frame!(self, AuthenticationResponse::Initialization(response));
                }
                Authentication::Challenge(x) => {
                    trace!("Authenticate::Challenge({x:?})");
                    let response = handler.on_challenge(x).await?;
                    write_frame!(self, AuthenticationResponse::Challenge(response));
                }
                Authentication::Verification(x) => {
                    trace!("Authenticate::Verify({x:?})");
                    let response = handler.on_verification(x).await?;
                    write_frame!(self, AuthenticationResponse::Verification(response));
                }
                Authentication::Info(x) => {
                    trace!("Authenticate::Info({x:?})");
                    handler.on_info(x).await?;
                }
                Authentication::Error(x) => {
                    trace!("Authenticate::Error({x:?})");
                    handler.on_error(x.clone()).await?;

                    if x.is_fatal() {
                        return Err(x.into_io_permission_denied());
                    }
                }
                Authentication::StartMethod(x) => {
                    trace!("Authenticate::StartMethod({x:?})");
                    handler.on_start_method(x).await?;
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
    async fn initialize(
        &mut self,
        initialization: Initialization,
    ) -> io::Result<InitializationResponse> {
        trace!("Authenticator::initialize({initialization:?})");
        write_frame!(self, Authentication::Initialization(initialization));
        let response = next_frame_as!(self, AuthenticationResponse, Initialization);
        Ok(response)
    }

    async fn challenge(&mut self, challenge: Challenge) -> io::Result<ChallengeResponse> {
        trace!("Authenticator::challenge({challenge:?})");
        write_frame!(self, Authentication::Challenge(challenge));
        let response = next_frame_as!(self, AuthenticationResponse, Challenge);
        Ok(response)
    }

    async fn verify(&mut self, verification: Verification) -> io::Result<VerificationResponse> {
        trace!("Authenticator::verify({verification:?})");
        write_frame!(self, Authentication::Verification(verification));
        let response = next_frame_as!(self, AuthenticationResponse, Verification);
        Ok(response)
    }

    async fn info(&mut self, info: Info) -> io::Result<()> {
        trace!("Authenticator::info({info:?})");
        write_frame!(self, Authentication::Info(info));
        Ok(())
    }

    async fn error(&mut self, error: Error) -> io::Result<()> {
        trace!("Authenticator::error({error:?})");
        write_frame!(self, Authentication::Error(error));
        Ok(())
    }

    async fn start_method(&mut self, start_method: StartMethod) -> io::Result<()> {
        trace!("Authenticator::start_method({start_method:?})");
        write_frame!(self, Authentication::StartMethod(start_method));
        Ok(())
    }

    async fn finished(&mut self) -> io::Result<()> {
        trace!("Authenticator::finished()");
        write_frame!(self, Authentication::Finished);
        Ok(())
    }
}
