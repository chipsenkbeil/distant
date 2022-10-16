use super::{msg::*, AuthHandler};
use crate::common::{utils, FramedTransport, Transport};
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
        let data = utils::serialize_to_vec(&$data)?;
        if log_enabled!(Level::Trace) {
            trace!("Writing data as frame: {data:?}");
        }

        $transport.write_frame(data).await?
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

        match utils::deserialize_from_slice::<$type>(frame.as_item()) {
            Ok(frame) => frame,
            Err(x) => {
                if log_enabled!(Level::Trace) {
                    trace!(
                        "Failed to deserialize frame item as {}: {:?}",
                        stringify!($type),
                        frame.as_item()
                    );
                }

                Err(x)?;
                unreachable!();
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;
    use tokio::sync::mpsc;

    macro_rules! auth_handler {
        (@no_challenge @no_verification @tx($tx:ident, $ty:ty) $($methods:item)*) => {
            auth_handler! {
                @tx($tx, $ty)

                async fn on_challenge(&mut self, _: Challenge) -> io::Result<ChallengeResponse> {
                    Err(io::Error::from(io::ErrorKind::Unsupported))
                }

                async fn on_verification(
                    &mut self,
                    _: Verification,
                ) -> io::Result<VerificationResponse> {
                    Err(io::Error::from(io::ErrorKind::Unsupported))
                }

                $($methods)*
            }
        };
        (@no_challenge @tx($tx:ident, $ty:ty) $($methods:item)*) => {
            auth_handler! {
                @tx($tx, $ty)

                async fn on_challenge(&mut self, _: Challenge) -> io::Result<ChallengeResponse> {
                    Err(io::Error::from(io::ErrorKind::Unsupported))
                }

                $($methods)*
            }
        };
        (@no_verification @tx($tx:ident, $ty:ty) $($methods:item)*) => {
            auth_handler! {
                @tx($tx, $ty)

                async fn on_verification(
                    &mut self,
                    _: Verification,
                ) -> io::Result<VerificationResponse> {
                    Err(io::Error::from(io::ErrorKind::Unsupported))
                }

                $($methods)*
            }
        };
        (@tx($tx:ident, $ty:ty) $($methods:item)*) => {{
            #[allow(dead_code)]
            struct __InlineAuthHandler {
                tx: mpsc::Sender<$ty>,
            }

            #[async_trait]
            impl AuthHandler for __InlineAuthHandler {
                $($methods)*
            }

            __InlineAuthHandler { tx: $tx }
        }};
    }

    #[test(tokio::test)]
    async fn authenticator_initialization_should_be_able_to_successfully_complete_round_trip() {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);
        let (tx, _) = mpsc::channel(1);

        let task = tokio::spawn(async move {
            t2.authenticate(auth_handler! {
                @no_challenge
                @no_verification
                @tx(tx, ())

                async fn on_initialization(
                    &mut self,
                    initialization: Initialization,
                ) -> io::Result<InitializationResponse> {
                    Ok(InitializationResponse {
                        methods: initialization.methods,
                    })
                }
            })
            .await
            .unwrap()
        });

        let response = t1
            .initialize(Initialization {
                methods: vec!["test method".to_string()].into_iter().collect(),
            })
            .await
            .unwrap();

        assert!(
            !task.is_finished(),
            "Auth handler unexpectedly finished without signal"
        );

        assert_eq!(
            response,
            InitializationResponse {
                methods: vec!["test method".to_string()].into_iter().collect()
            }
        );
    }

    #[test(tokio::test)]
    async fn authenticator_challenge_should_be_able_to_successfully_complete_round_trip() {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);
        let (tx, _) = mpsc::channel(1);

        let task = tokio::spawn(async move {
            t2.authenticate(auth_handler! {
                @no_verification
                @tx(tx, ())

                async fn on_challenge(&mut self, challenge: Challenge) -> io::Result<ChallengeResponse> {
                    assert_eq!(challenge.questions, vec![Question {
                        label: "label".to_string(),
                        text: "text".to_string(),
                        options: vec![("question_key".to_string(), "question_value".to_string())]
                            .into_iter()
                            .collect(),
                    }]);
                    assert_eq!(
                        challenge.options,
                        vec![("key".to_string(), "value".to_string())].into_iter().collect(),
                    );
                    Ok(ChallengeResponse {
                        answers: vec!["some answer".to_string()].into_iter().collect(),
                    })
                }
            })
            .await
            .unwrap()
        });

        let response = t1
            .challenge(Challenge {
                questions: vec![Question {
                    label: "label".to_string(),
                    text: "text".to_string(),
                    options: vec![("question_key".to_string(), "question_value".to_string())]
                        .into_iter()
                        .collect(),
                }],
                options: vec![("key".to_string(), "value".to_string())]
                    .into_iter()
                    .collect(),
            })
            .await
            .unwrap();

        assert!(
            !task.is_finished(),
            "Auth handler unexpectedly finished without signal"
        );

        assert_eq!(
            response,
            ChallengeResponse {
                answers: vec!["some answer".to_string()],
            }
        );
    }

    #[test(tokio::test)]
    async fn authenticator_verification_should_be_able_to_successfully_complete_round_trip() {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);
        let (tx, _) = mpsc::channel(1);

        let task = tokio::spawn(async move {
            t2.authenticate(auth_handler! {
                @no_challenge
                @tx(tx, ())

                async fn on_verification(
                    &mut self,
                    verification: Verification,
                ) -> io::Result<VerificationResponse> {
                    assert_eq!(verification.kind, VerificationKind::Host);
                    assert_eq!(verification.text, "some text");
                    Ok(VerificationResponse {
                        valid: true,
                    })
                }
            })
            .await
            .unwrap()
        });

        let response = t1
            .verify(Verification {
                kind: VerificationKind::Host,
                text: "some text".to_string(),
            })
            .await
            .unwrap();

        assert!(
            !task.is_finished(),
            "Auth handler unexpectedly finished without signal"
        );

        assert_eq!(response, VerificationResponse { valid: true });
    }

    #[test(tokio::test)]
    async fn authenticator_info_should_be_able_to_be_sent_to_auth_handler() {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);
        let (tx, mut rx) = mpsc::channel(1);

        let task = tokio::spawn(async move {
            t2.authenticate(auth_handler! {
                @no_challenge
                @no_verification
                @tx(tx, Info)

                async fn on_info(
                    &mut self,
                    info: Info,
                ) -> io::Result<()> {
                    self.tx.send(info).await.unwrap();
                    Ok(())
                }
            })
            .await
            .unwrap()
        });

        t1.info(Info {
            text: "some text".to_string(),
        })
        .await
        .unwrap();

        assert_eq!(
            rx.recv().await.unwrap(),
            Info {
                text: "some text".to_string()
            }
        );

        assert!(
            !task.is_finished(),
            "Auth handler unexpectedly finished without signal"
        );
    }

    #[test(tokio::test)]
    async fn authenticator_error_should_be_able_to_be_sent_to_auth_handler() {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);
        let (tx, mut rx) = mpsc::channel(1);

        let task = tokio::spawn(async move {
            t2.authenticate(auth_handler! {
                @no_challenge
                @no_verification
                @tx(tx, Error)

                async fn on_error(&mut self, error: Error) -> io::Result<()> {
                    self.tx.send(error).await.unwrap();
                    Ok(())
                }
            })
            .await
            .unwrap()
        });

        t1.error(Error {
            kind: ErrorKind::Error,
            text: "some text".to_string(),
        })
        .await
        .unwrap();

        assert_eq!(
            rx.recv().await.unwrap(),
            Error {
                kind: ErrorKind::Error,
                text: "some text".to_string(),
            }
        );

        assert!(
            !task.is_finished(),
            "Auth handler unexpectedly finished without signal"
        );
    }

    #[test(tokio::test)]
    async fn auth_handler_received_error_should_fail_auth_handler_if_fatal() {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);
        let (tx, mut rx) = mpsc::channel(1);

        let task = tokio::spawn(async move {
            t2.authenticate(auth_handler! {
                @no_challenge
                @no_verification
                @tx(tx, Error)

                async fn on_error(&mut self, error: Error) -> io::Result<()> {
                    self.tx.send(error).await.unwrap();
                    Ok(())
                }
            })
            .await
            .unwrap()
        });

        t1.error(Error {
            kind: ErrorKind::Fatal,
            text: "some text".to_string(),
        })
        .await
        .unwrap();

        assert_eq!(
            rx.recv().await.unwrap(),
            Error {
                kind: ErrorKind::Fatal,
                text: "some text".to_string(),
            }
        );

        // Verify that the handler exited with an error
        task.await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn authenticator_start_method_should_be_able_to_be_sent_to_auth_handler() {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);
        let (tx, mut rx) = mpsc::channel(1);

        let task = tokio::spawn(async move {
            t2.authenticate(auth_handler! {
                @no_challenge
                @no_verification
                @tx(tx, StartMethod)

                async fn on_start_method(&mut self, start_method: StartMethod) -> io::Result<()> {
                    self.tx.send(start_method).await.unwrap();
                    Ok(())
                }
            })
            .await
            .unwrap()
        });

        t1.start_method(StartMethod {
            method: "some method".to_string(),
        })
        .await
        .unwrap();

        assert_eq!(
            rx.recv().await.unwrap(),
            StartMethod {
                method: "some method".to_string()
            }
        );

        assert!(
            !task.is_finished(),
            "Auth handler unexpectedly finished without signal"
        );
    }

    #[test(tokio::test)]
    async fn authenticator_finished_should_be_able_to_be_sent_to_auth_handler() {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);
        let (tx, _) = mpsc::channel(1);

        let task = tokio::spawn(async move {
            t2.authenticate(auth_handler! {
                @no_challenge
                @no_verification
                @tx(tx, ())

                async fn on_finished(&mut self) -> io::Result<()> {
                    Ok(())
                }
            })
            .await
            .unwrap()
        });

        t1.finished().await.unwrap();

        // Finished should signal that the handler completed successfully
        task.await.unwrap();
    }
}
