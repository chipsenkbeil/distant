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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tokio::sync::mpsc;

    use crate::net::common::Response;
    use crate::net::server::ServerReply;

    /// Creates a ManagerAuthenticator and returns it along with the receiver
    /// that captures outgoing ManagerResponse messages.
    fn make_authenticator() -> (
        ManagerAuthenticator,
        mpsc::UnboundedReceiver<Response<ManagerResponse>>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        let reply = ServerReply {
            origin_id: String::from("auth-test"),
            tx,
        };
        let auth = ManagerAuthenticator {
            reply,
            registry: Arc::new(RwLock::new(HashMap::new())),
        };
        (auth, rx)
    }

    // ---------------------------------------------------------------
    // fire() - sends without expecting a reply
    // ---------------------------------------------------------------

    #[test_log::test(tokio::test)]
    async fn fire_sends_authenticate_message() {
        let (auth, mut rx) = make_authenticator();
        auth.fire(Authentication::Finished).unwrap();

        let resp = rx.recv().await.unwrap();
        match resp.payload {
            ManagerResponse::Authenticate { msg, .. } => {
                assert_eq!(msg, Authentication::Finished);
            }
            other => panic!("Expected Authenticate, got {other:?}"),
        }
    }

    #[test_log::test(tokio::test)]
    async fn fire_does_not_store_callback_in_registry() {
        let (auth, _rx) = make_authenticator();
        auth.fire(Authentication::Finished).unwrap();

        let registry = auth.registry.read().await;
        assert!(registry.is_empty());
    }

    #[test_log::test(tokio::test)]
    async fn fire_fails_when_receiver_dropped() {
        let (auth, rx) = make_authenticator();
        drop(rx);
        let result = auth.fire(Authentication::Finished);
        assert!(result.is_err());
    }

    // ---------------------------------------------------------------
    // send() - sends and waits for a callback response
    // ---------------------------------------------------------------

    #[test_log::test(tokio::test)]
    async fn send_stores_callback_and_receives_response() {
        let (auth, mut rx) = make_authenticator();

        let send_task = tokio::spawn({
            let auth = auth.clone();
            async move { auth.send(Authentication::Finished).await }
        });

        // Receive the outgoing message and extract the id
        let resp = rx.recv().await.unwrap();
        let id = match resp.payload {
            ManagerResponse::Authenticate { id, .. } => id,
            other => panic!("Expected Authenticate, got {other:?}"),
        };

        // Deliver the response via the registry callback
        {
            let mut registry = auth.registry.write().await;
            let sender = registry.remove(&id).unwrap();
            sender
                .send(AuthenticationResponse::Initialization(
                    InitializationResponse {
                        methods: vec![String::from("none")],
                    },
                ))
                .unwrap();
        }

        // The send() future should now resolve
        let result = send_task.await.unwrap().unwrap();
        assert_eq!(
            result,
            AuthenticationResponse::Initialization(InitializationResponse {
                methods: vec![String::from("none")],
            })
        );
    }

    #[test_log::test(tokio::test)]
    async fn send_fails_when_receiver_dropped() {
        let (auth, rx) = make_authenticator();
        drop(rx);
        let result = auth.send(Authentication::Finished).await;
        assert!(result.is_err());
    }

    #[test_log::test(tokio::test)]
    async fn send_fails_when_callback_sender_dropped() {
        let (auth, mut rx) = make_authenticator();

        let send_task = tokio::spawn({
            let auth = auth.clone();
            async move { auth.send(Authentication::Finished).await }
        });

        // Receive the outgoing message to get the id
        let resp = rx.recv().await.unwrap();
        let id = match resp.payload {
            ManagerResponse::Authenticate { id, .. } => id,
            other => panic!("Expected Authenticate, got {other:?}"),
        };

        // Remove and drop the sender without sending a response
        {
            let mut registry = auth.registry.write().await;
            let sender = registry.remove(&id).unwrap();
            drop(sender);
        }

        // The send() future should resolve with an error
        let result = send_task.await.unwrap();
        assert!(result.is_err());
    }

    // ---------------------------------------------------------------
    // Authenticator::initialize
    // ---------------------------------------------------------------

    #[test_log::test(tokio::test)]
    async fn initialize_sends_initialization_and_returns_response() {
        let (mut auth, mut rx) = make_authenticator();

        let task = tokio::spawn({
            let registry = Arc::clone(&auth.registry);
            async move {
                let resp = rx.recv().await.unwrap();
                let id = match resp.payload {
                    ManagerResponse::Authenticate { id, msg } => {
                        assert!(matches!(msg, Authentication::Initialization(_)));
                        id
                    }
                    other => panic!("Expected Authenticate, got {other:?}"),
                };
                let mut reg = registry.write().await;
                let sender = reg.remove(&id).unwrap();
                sender
                    .send(AuthenticationResponse::Initialization(
                        InitializationResponse {
                            methods: vec![String::from("static_key")],
                        },
                    ))
                    .unwrap();
            }
        });

        let result = auth
            .initialize(Initialization {
                methods: vec![String::from("static_key"), String::from("none")],
            })
            .await
            .unwrap();

        assert_eq!(result.methods, vec![String::from("static_key")]);
        task.await.unwrap();
    }

    #[test_log::test(tokio::test)]
    async fn initialize_returns_error_on_wrong_response_type() {
        let (mut auth, mut rx) = make_authenticator();

        let task = tokio::spawn({
            let registry = Arc::clone(&auth.registry);
            async move {
                let resp = rx.recv().await.unwrap();
                let id = match resp.payload {
                    ManagerResponse::Authenticate { id, .. } => id,
                    other => panic!("Expected Authenticate, got {other:?}"),
                };
                let mut reg = registry.write().await;
                let sender = reg.remove(&id).unwrap();
                // Send wrong response type (Challenge instead of Initialization)
                sender
                    .send(AuthenticationResponse::Challenge(ChallengeResponse {
                        answers: vec![],
                    }))
                    .unwrap();
            }
        });

        let result = auth
            .initialize(Initialization {
                methods: vec![String::from("test")],
            })
            .await;

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Unexpected"));
        task.await.unwrap();
    }

    // ---------------------------------------------------------------
    // Authenticator::challenge
    // ---------------------------------------------------------------

    #[test_log::test(tokio::test)]
    async fn challenge_sends_challenge_and_returns_response() {
        let (mut auth, mut rx) = make_authenticator();

        let task = tokio::spawn({
            let registry = Arc::clone(&auth.registry);
            async move {
                let resp = rx.recv().await.unwrap();
                let id = match resp.payload {
                    ManagerResponse::Authenticate { id, msg } => {
                        assert!(matches!(msg, Authentication::Challenge(_)));
                        id
                    }
                    other => panic!("Expected Authenticate, got {other:?}"),
                };
                let mut reg = registry.write().await;
                let sender = reg.remove(&id).unwrap();
                sender
                    .send(AuthenticationResponse::Challenge(ChallengeResponse {
                        answers: vec![String::from("my_password")],
                    }))
                    .unwrap();
            }
        });

        let result = auth
            .challenge(Challenge {
                questions: vec![],
                options: HashMap::new(),
            })
            .await
            .unwrap();

        assert_eq!(result.answers, vec![String::from("my_password")]);
        task.await.unwrap();
    }

    #[test_log::test(tokio::test)]
    async fn challenge_returns_error_on_wrong_response_type() {
        let (mut auth, mut rx) = make_authenticator();

        let task = tokio::spawn({
            let registry = Arc::clone(&auth.registry);
            async move {
                let resp = rx.recv().await.unwrap();
                let id = match resp.payload {
                    ManagerResponse::Authenticate { id, .. } => id,
                    other => panic!("Expected Authenticate, got {other:?}"),
                };
                let mut reg = registry.write().await;
                let sender = reg.remove(&id).unwrap();
                sender
                    .send(AuthenticationResponse::Verification(VerificationResponse {
                        valid: true,
                    }))
                    .unwrap();
            }
        });

        let result = auth
            .challenge(Challenge {
                questions: vec![],
                options: HashMap::new(),
            })
            .await;

        assert!(result.is_err());
        task.await.unwrap();
    }

    // ---------------------------------------------------------------
    // Authenticator::verify
    // ---------------------------------------------------------------

    #[test_log::test(tokio::test)]
    async fn verify_sends_verification_and_returns_response() {
        let (mut auth, mut rx) = make_authenticator();

        let task = tokio::spawn({
            let registry = Arc::clone(&auth.registry);
            async move {
                let resp = rx.recv().await.unwrap();
                let id = match resp.payload {
                    ManagerResponse::Authenticate { id, msg } => {
                        assert!(matches!(msg, Authentication::Verification(_)));
                        id
                    }
                    other => panic!("Expected Authenticate, got {other:?}"),
                };
                let mut reg = registry.write().await;
                let sender = reg.remove(&id).unwrap();
                sender
                    .send(AuthenticationResponse::Verification(VerificationResponse {
                        valid: true,
                    }))
                    .unwrap();
            }
        });

        let result = auth
            .verify(Verification {
                kind: VerificationKind::Host,
                text: String::from("fingerprint abc"),
            })
            .await
            .unwrap();

        assert!(result.valid);
        task.await.unwrap();
    }

    #[test_log::test(tokio::test)]
    async fn verify_returns_error_on_wrong_response_type() {
        let (mut auth, mut rx) = make_authenticator();

        let task = tokio::spawn({
            let registry = Arc::clone(&auth.registry);
            async move {
                let resp = rx.recv().await.unwrap();
                let id = match resp.payload {
                    ManagerResponse::Authenticate { id, .. } => id,
                    other => panic!("Expected Authenticate, got {other:?}"),
                };
                let mut reg = registry.write().await;
                let sender = reg.remove(&id).unwrap();
                sender
                    .send(AuthenticationResponse::Initialization(
                        InitializationResponse { methods: vec![] },
                    ))
                    .unwrap();
            }
        });

        let result = auth
            .verify(Verification {
                kind: VerificationKind::Host,
                text: String::from("test"),
            })
            .await;

        assert!(result.is_err());
        task.await.unwrap();
    }

    // ---------------------------------------------------------------
    // Authenticator::info (fire-and-forget)
    // ---------------------------------------------------------------

    #[test_log::test(tokio::test)]
    async fn info_sends_info_message() {
        let (mut auth, mut rx) = make_authenticator();

        auth.info(Info {
            text: String::from("hello"),
        })
        .await
        .unwrap();

        let resp = rx.recv().await.unwrap();
        match resp.payload {
            ManagerResponse::Authenticate { msg, .. } => {
                assert_eq!(
                    msg,
                    Authentication::Info(Info {
                        text: String::from("hello"),
                    })
                );
            }
            other => panic!("Expected Authenticate, got {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // Authenticator::error (fire-and-forget)
    // ---------------------------------------------------------------

    #[test_log::test(tokio::test)]
    async fn error_sends_error_message() {
        let (mut auth, mut rx) = make_authenticator();

        auth.error(Error::fatal("something broke")).await.unwrap();

        let resp = rx.recv().await.unwrap();
        match resp.payload {
            ManagerResponse::Authenticate { msg, .. } => {
                assert_eq!(msg, Authentication::Error(Error::fatal("something broke")));
            }
            other => panic!("Expected Authenticate, got {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // Authenticator::start_method (fire-and-forget)
    // ---------------------------------------------------------------

    #[test_log::test(tokio::test)]
    async fn start_method_sends_start_method_message() {
        let (mut auth, mut rx) = make_authenticator();

        auth.start_method(StartMethod {
            method: String::from("static_key"),
        })
        .await
        .unwrap();

        let resp = rx.recv().await.unwrap();
        match resp.payload {
            ManagerResponse::Authenticate { msg, .. } => {
                assert_eq!(
                    msg,
                    Authentication::StartMethod(StartMethod {
                        method: String::from("static_key"),
                    })
                );
            }
            other => panic!("Expected Authenticate, got {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // Authenticator::finished (fire-and-forget)
    // ---------------------------------------------------------------

    #[test_log::test(tokio::test)]
    async fn finished_sends_finished_message() {
        let (mut auth, mut rx) = make_authenticator();

        auth.finished().await.unwrap();

        let resp = rx.recv().await.unwrap();
        match resp.payload {
            ManagerResponse::Authenticate { msg, .. } => {
                assert_eq!(msg, Authentication::Finished);
            }
            other => panic!("Expected Authenticate, got {other:?}"),
        }
    }

    // ---------------------------------------------------------------
    // fire-and-forget methods fail when receiver is dropped
    // ---------------------------------------------------------------

    #[test_log::test(tokio::test)]
    async fn info_fails_when_receiver_dropped() {
        let (mut auth, rx) = make_authenticator();
        drop(rx);
        let result = auth
            .info(Info {
                text: String::from("test"),
            })
            .await;
        assert!(result.is_err());
    }

    #[test_log::test(tokio::test)]
    async fn error_fails_when_receiver_dropped() {
        let (mut auth, rx) = make_authenticator();
        drop(rx);
        let result = auth.error(Error::fatal("test")).await;
        assert!(result.is_err());
    }

    #[test_log::test(tokio::test)]
    async fn start_method_fails_when_receiver_dropped() {
        let (mut auth, rx) = make_authenticator();
        drop(rx);
        let result = auth
            .start_method(StartMethod {
                method: String::from("test"),
            })
            .await;
        assert!(result.is_err());
    }

    #[test_log::test(tokio::test)]
    async fn finished_fails_when_receiver_dropped() {
        let (mut auth, rx) = make_authenticator();
        drop(rx);
        let result = auth.finished().await;
        assert!(result.is_err());
    }
}
