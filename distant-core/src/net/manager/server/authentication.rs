use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;

use crate::auth::msg::*;
use crate::auth::Authenticator;
use tokio::sync::{oneshot, RwLock};

use crate::net::manager::data::{ManagerAuthenticationId, ManagerResponse};
use crate::net::server::ServerReply;

/// Implementation of [`Authenticator`] used by a manger to perform authentication with
/// remote servers it is managing.
#[derive(Clone)]
pub struct ManagerAuthenticator {
    /// Used to communicate authentication requests
    pub(super) reply: ServerReply<ManagerResponse>,

    /// Used to store one-way response senders that are used to return callbacks
    pub(super) registry:
        Arc<RwLock<HashMap<ManagerAuthenticationId, oneshot::Sender<AuthenticationResponse>>>>,
}

impl ManagerAuthenticator {
    /// Sends an [`Authentication`] `msg` that expects a reply, storing a callback.
    async fn send(&self, msg: Authentication) -> io::Result<AuthenticationResponse> {
        let (tx, rx) = oneshot::channel();
        let id = rand::random();

        self.registry.write().await.insert(id, tx);
        self.reply.send(ManagerResponse::Authenticate { id, msg })?;
        rx.await.map_err(io::Error::other)
    }

    /// Sends an [`Authentication`] `msg` without expecting a reply. No callback is stored.
    fn fire(&self, msg: Authentication) -> io::Result<()> {
        let id = rand::random();
        self.reply.send(ManagerResponse::Authenticate { id, msg })?;
        Ok(())
    }
}

/// Represents an interface for submitting challenges for authentication.
impl Authenticator for ManagerAuthenticator {
    fn initialize<'a>(
        &'a mut self,
        initialization: Initialization,
    ) -> Pin<Box<dyn Future<Output = io::Result<InitializationResponse>> + Send + 'a>> {
        Box::pin(async move {
            match self
                .send(Authentication::Initialization(initialization))
                .await
            {
                Ok(AuthenticationResponse::Initialization(x)) => Ok(x),
                Ok(x) => Err(io::Error::other(format!("Unexpected response: {x:?}"))),
                Err(x) => Err(x),
            }
        })
    }

    fn challenge<'a>(
        &'a mut self,
        challenge: Challenge,
    ) -> Pin<Box<dyn Future<Output = io::Result<ChallengeResponse>> + Send + 'a>> {
        Box::pin(async move {
            match self.send(Authentication::Challenge(challenge)).await {
                Ok(AuthenticationResponse::Challenge(x)) => Ok(x),
                Ok(x) => Err(io::Error::other(format!("Unexpected response: {x:?}"))),
                Err(x) => Err(x),
            }
        })
    }

    fn verify<'a>(
        &'a mut self,
        verification: Verification,
    ) -> Pin<Box<dyn Future<Output = io::Result<VerificationResponse>> + Send + 'a>> {
        Box::pin(async move {
            match self.send(Authentication::Verification(verification)).await {
                Ok(AuthenticationResponse::Verification(x)) => Ok(x),
                Ok(x) => Err(io::Error::other(format!("Unexpected response: {x:?}"))),
                Err(x) => Err(x),
            }
        })
    }

    fn info<'a>(
        &'a mut self,
        info: Info,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move { self.fire(Authentication::Info(info)) })
    }

    fn error<'a>(
        &'a mut self,
        error: Error,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move { self.fire(Authentication::Error(error)) })
    }

    fn start_method<'a>(
        &'a mut self,
        start_method: StartMethod,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move { self.fire(Authentication::StartMethod(start_method)) })
    }

    fn finished<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move { self.fire(Authentication::Finished) })
    }
}
