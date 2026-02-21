use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use async_trait::async_trait;
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
#[async_trait]
impl Authenticator for ManagerAuthenticator {
    async fn initialize(
        &mut self,
        initialization: Initialization,
    ) -> io::Result<InitializationResponse> {
        match self
            .send(Authentication::Initialization(initialization))
            .await
        {
            Ok(AuthenticationResponse::Initialization(x)) => Ok(x),
            Ok(x) => Err(io::Error::other(format!("Unexpected response: {x:?}"))),
            Err(x) => Err(x),
        }
    }

    async fn challenge(&mut self, challenge: Challenge) -> io::Result<ChallengeResponse> {
        match self.send(Authentication::Challenge(challenge)).await {
            Ok(AuthenticationResponse::Challenge(x)) => Ok(x),
            Ok(x) => Err(io::Error::other(format!("Unexpected response: {x:?}"))),
            Err(x) => Err(x),
        }
    }

    async fn verify(&mut self, verification: Verification) -> io::Result<VerificationResponse> {
        match self.send(Authentication::Verification(verification)).await {
            Ok(AuthenticationResponse::Verification(x)) => Ok(x),
            Ok(x) => Err(io::Error::other(format!("Unexpected response: {x:?}"))),
            Err(x) => Err(x),
        }
    }

    async fn info(&mut self, info: Info) -> io::Result<()> {
        self.fire(Authentication::Info(info))
    }

    async fn error(&mut self, error: Error) -> io::Result<()> {
        self.fire(Authentication::Error(error))
    }

    async fn start_method(&mut self, start_method: StartMethod) -> io::Result<()> {
        self.fire(Authentication::StartMethod(start_method))
    }

    async fn finished(&mut self) -> io::Result<()> {
        self.fire(Authentication::Finished)
    }
}
