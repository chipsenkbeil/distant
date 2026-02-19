use std::io;

use async_trait::async_trait;
use distant_auth::msg::*;
use distant_auth::{AuthHandler, Authenticate, Authenticator};
use log::*;

use crate::common::{utils, FramedTransport, Transport};

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
            io::Error::new(
                io::ErrorKind::UnexpectedEof,
                concat!(
                    "Transport closed early waiting for frame of type ",
                    stringify!($type),
                ),
            )
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
    T: Transport,
{
    async fn authenticate(&mut self, mut handler: impl AuthHandler) -> io::Result<()> {
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
                    handler.on_finished().await?;
                    return Ok(());
                }
            }
        }
    }
}

#[async_trait]
impl<T> Authenticator for FramedTransport<T>
where
    T: Transport,
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
    use distant_auth::tests::TestAuthHandler;
    use test_log::test;
    use tokio::sync::mpsc;

    use super::*;

    #[test(tokio::test)]
    async fn authenticator_initialization_should_be_able_to_successfully_complete_round_trip() {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        let task = tokio::spawn(async move {
            t2.authenticate(TestAuthHandler {
                on_initialization: Box::new(|x| Ok(InitializationResponse { methods: x.methods })),
                ..Default::default()
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

        let task = tokio::spawn(async move {
            t2.authenticate(TestAuthHandler {
                on_challenge: Box::new(|challenge| {
                    assert_eq!(
                        challenge.questions,
                        vec![Question {
                            label: "label".to_string(),
                            text: "text".to_string(),
                            options: vec![(
                                "question_key".to_string(),
                                "question_value".to_string()
                            )]
                            .into_iter()
                            .collect(),
                        }]
                    );
                    assert_eq!(
                        challenge.options,
                        vec![("key".to_string(), "value".to_string())]
                            .into_iter()
                            .collect(),
                    );
                    Ok(ChallengeResponse {
                        answers: vec!["some answer".to_string()].into_iter().collect(),
                    })
                }),
                ..Default::default()
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

        let task = tokio::spawn(async move {
            t2.authenticate(TestAuthHandler {
                on_verification: Box::new(|verification| {
                    assert_eq!(verification.kind, VerificationKind::Host);
                    assert_eq!(verification.text, "some text");
                    Ok(VerificationResponse { valid: true })
                }),
                ..Default::default()
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
            t2.authenticate(TestAuthHandler {
                on_info: Box::new(move |info| {
                    tx.try_send(info).unwrap();
                    Ok(())
                }),
                ..Default::default()
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
            t2.authenticate(TestAuthHandler {
                on_error: Box::new(move |error| {
                    tx.try_send(error).unwrap();
                    Ok(())
                }),
                ..Default::default()
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
            t2.authenticate(TestAuthHandler {
                on_error: Box::new(move |error| {
                    tx.try_send(error).unwrap();
                    Ok(())
                }),
                ..Default::default()
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
            t2.authenticate(TestAuthHandler {
                on_start_method: Box::new(move |start_method| {
                    tx.try_send(start_method).unwrap();
                    Ok(())
                }),
                ..Default::default()
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
        let (tx, mut rx) = mpsc::channel(1);

        let task = tokio::spawn(async move {
            t2.authenticate(TestAuthHandler {
                on_finished: Box::new(move || {
                    tx.try_send(()).unwrap();
                    Ok(())
                }),
                ..Default::default()
            })
            .await
            .unwrap()
        });

        t1.finished().await.unwrap();

        // Verify that the callback was triggered
        rx.recv().await.unwrap();

        // Finished should signal that the handler completed successfully
        task.await.unwrap();
    }
}
