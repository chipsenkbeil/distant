use crate::{
    utils, Auth, AuthErrorKind, AuthQuestion, AuthRequest, AuthResponse, AuthVerifyKind, Client,
    Codec, Handshake, XChaCha20Poly1305Codec,
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
        questions: Vec<AuthQuestion>,
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
            io::Error::new(
                io::ErrorKind::Other,
                "Handshake must be performed first (client encrypt message)",
            )
        })?;

        let mut encryped_payload = BytesMut::new();
        let payload = utils::serialize_to_vec(payload)?;
        codec.encode(&payload, &mut encryped_payload)?;
        Ok(encryped_payload.freeze().to_vec())
    }

    fn decrypt_and_deserialize(&mut self, payload: &[u8]) -> io::Result<AuthResponse> {
        let codec = self.codec.as_mut().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Other,
                "Handshake must be performed first (client decrypt message)",
            )
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Client, FramedTransport, Request, Response, TypedAsyncRead, TypedAsyncWrite};
    use serde::{de::DeserializeOwned, Serialize};

    const TIMEOUT_MILLIS: u64 = 100;

    #[tokio::test]
    async fn handshake_should_fail_if_get_unexpected_response_from_server() {
        let (t, mut server) = FramedTransport::make_test_pair();
        let mut client = AuthClient::from(Client::from_framed_transport(t).unwrap());

        // We start a separate task for the client to avoid blocking since
        // we also need to receive the client's request and respond
        let task = tokio::spawn(async move { client.handshake().await });

        // Get the request, but send a bad response
        let request: Request<Auth> = server.read().await.unwrap().unwrap();
        match request.payload {
            Auth::Handshake { .. } => server
                .write(Response::new(
                    request.id,
                    Auth::Msg {
                        encrypted_payload: Vec::new(),
                    },
                ))
                .await
                .unwrap(),
            _ => panic!("Server received unexpected payload"),
        }

        let result = task.await.unwrap();
        assert!(result.is_err(), "Handshake succeeded unexpectedly")
    }

    #[tokio::test]
    async fn challenge_should_fail_if_handshake_not_finished() {
        let (t, mut server) = FramedTransport::make_test_pair();
        let mut client = AuthClient::from(Client::from_framed_transport(t).unwrap());

        // We start a separate task for the client to avoid blocking since
        // we also need to receive the client's request and respond
        let task = tokio::spawn(async move { client.challenge(Vec::new(), HashMap::new()).await });

        // Wait for a request, failing if we get one as the failure
        // should have prevented sending anything, but we should
        tokio::select! {
            x = TypedAsyncRead::<Request<Auth>>::read(&mut server) => {
                match x {
                    Ok(Some(x)) => panic!("Unexpectedly resolved: {:?}", x),
                    Ok(None) => {},
                    Err(x) => panic!("Unexpectedly failed on server side: {}", x),
                }
            },
            _ = wait_ms(TIMEOUT_MILLIS) => {
                panic!("Should have gotten server closure as part of client exit");
            }
        }

        // Verify that we got an error with the method
        let result = task.await.unwrap();
        assert!(result.is_err(), "Challenge succeeded unexpectedly")
    }

    #[tokio::test]
    async fn challenge_should_fail_if_receive_wrong_response() {
        let (t, mut server) = FramedTransport::make_test_pair();
        let mut client = AuthClient::from(Client::from_framed_transport(t).unwrap());

        // We start a separate task for the client to avoid blocking since
        // we also need to receive the client's request and respond
        let task = tokio::spawn(async move {
            client.handshake().await.unwrap();
            client
                .challenge(
                    vec![
                        AuthQuestion::new("question1".to_string()),
                        AuthQuestion {
                            text: "question2".to_string(),
                            extra: vec![("key2".to_string(), "value2".to_string())]
                                .into_iter()
                                .collect(),
                        },
                    ],
                    vec![("key".to_string(), "value".to_string())]
                        .into_iter()
                        .collect(),
                )
                .await
        });

        // Wait for a handshake request and set up our encryption codec
        let request: Request<Auth> = server.read().await.unwrap().unwrap();
        let mut codec = match request.payload {
            Auth::Handshake { public_key, salt } => {
                let handshake = Handshake::default();
                let key = handshake.handshake(public_key, salt).unwrap();
                server
                    .write(Response::new(
                        request.id,
                        Auth::Handshake {
                            public_key: handshake.pk_bytes(),
                            salt: *handshake.salt(),
                        },
                    ))
                    .await
                    .unwrap();
                XChaCha20Poly1305Codec::new(&key)
            }
            _ => panic!("Server received unexpected payload"),
        };

        // Wait for a challenge request and send back wrong response
        let request: Request<Auth> = server.read().await.unwrap().unwrap();
        match request.payload {
            Auth::Msg { encrypted_payload } => {
                match decrypt_and_deserialize(&mut codec, &encrypted_payload).unwrap() {
                    AuthRequest::Challenge { .. } => {
                        server
                            .write(Response::new(
                                request.id,
                                Auth::Msg {
                                    encrypted_payload: serialize_and_encrypt(
                                        &mut codec,
                                        &AuthResponse::Verify { valid: true },
                                    )
                                    .unwrap(),
                                },
                            ))
                            .await
                            .unwrap();
                    }
                    _ => panic!("Server received wrong request type"),
                }
            }
            _ => panic!("Server received unexpected payload"),
        };

        // Verify that we got an error with the method
        let result = task.await.unwrap();
        assert!(result.is_err(), "Challenge succeeded unexpectedly")
    }

    #[tokio::test]
    async fn challenge_should_return_answers_received_from_server() {
        let (t, mut server) = FramedTransport::make_test_pair();
        let mut client = AuthClient::from(Client::from_framed_transport(t).unwrap());

        // We start a separate task for the client to avoid blocking since
        // we also need to receive the client's request and respond
        let task = tokio::spawn(async move {
            client.handshake().await.unwrap();
            client
                .challenge(
                    vec![
                        AuthQuestion::new("question1".to_string()),
                        AuthQuestion {
                            text: "question2".to_string(),
                            extra: vec![("key2".to_string(), "value2".to_string())]
                                .into_iter()
                                .collect(),
                        },
                    ],
                    vec![("key".to_string(), "value".to_string())]
                        .into_iter()
                        .collect(),
                )
                .await
        });

        // Wait for a handshake request and set up our encryption codec
        let request: Request<Auth> = server.read().await.unwrap().unwrap();
        let mut codec = match request.payload {
            Auth::Handshake { public_key, salt } => {
                let handshake = Handshake::default();
                let key = handshake.handshake(public_key, salt).unwrap();
                server
                    .write(Response::new(
                        request.id,
                        Auth::Handshake {
                            public_key: handshake.pk_bytes(),
                            salt: *handshake.salt(),
                        },
                    ))
                    .await
                    .unwrap();
                XChaCha20Poly1305Codec::new(&key)
            }
            _ => panic!("Server received unexpected payload"),
        };

        // Wait for a challenge request and send back wrong response
        let request: Request<Auth> = server.read().await.unwrap().unwrap();
        match request.payload {
            Auth::Msg { encrypted_payload } => {
                match decrypt_and_deserialize(&mut codec, &encrypted_payload).unwrap() {
                    AuthRequest::Challenge { questions, extra } => {
                        assert_eq!(
                            questions,
                            vec![
                                AuthQuestion::new("question1".to_string()),
                                AuthQuestion {
                                    text: "question2".to_string(),
                                    extra: vec![("key2".to_string(), "value2".to_string())]
                                        .into_iter()
                                        .collect(),
                                },
                            ],
                        );

                        assert_eq!(
                            extra,
                            vec![("key".to_string(), "value".to_string())]
                                .into_iter()
                                .collect(),
                        );

                        server
                            .write(Response::new(
                                request.id,
                                Auth::Msg {
                                    encrypted_payload: serialize_and_encrypt(
                                        &mut codec,
                                        &AuthResponse::Challenge {
                                            answers: vec![
                                                "answer1".to_string(),
                                                "answer2".to_string(),
                                            ],
                                        },
                                    )
                                    .unwrap(),
                                },
                            ))
                            .await
                            .unwrap();
                    }
                    _ => panic!("Server received wrong request type"),
                }
            }
            _ => panic!("Server received unexpected payload"),
        };

        // Verify that we got the right results
        let answers = task.await.unwrap().unwrap();
        assert_eq!(answers, vec!["answer1".to_string(), "answer2".to_string()]);
    }

    #[tokio::test]
    async fn verify_should_fail_if_handshake_not_finished() {
        let (t, mut server) = FramedTransport::make_test_pair();
        let mut client = AuthClient::from(Client::from_framed_transport(t).unwrap());

        // We start a separate task for the client to avoid blocking since
        // we also need to receive the client's request and respond
        let task = tokio::spawn(async move {
            client
                .verify(AuthVerifyKind::Host, "some text".to_string())
                .await
        });

        // Wait for a request, failing if we get one as the failure
        // should have prevented sending anything, but we should
        tokio::select! {
            x = TypedAsyncRead::<Request<Auth>>::read(&mut server) => {
                match x {
                    Ok(Some(x)) => panic!("Unexpectedly resolved: {:?}", x),
                    Ok(None) => {},
                    Err(x) => panic!("Unexpectedly failed on server side: {}", x),
                }
            },
            _ = wait_ms(TIMEOUT_MILLIS) => {
                panic!("Should have gotten server closure as part of client exit");
            }
        }

        // Verify that we got an error with the method
        let result = task.await.unwrap();
        assert!(result.is_err(), "Verify succeeded unexpectedly")
    }

    #[tokio::test]
    async fn verify_should_fail_if_receive_wrong_response() {
        let (t, mut server) = FramedTransport::make_test_pair();
        let mut client = AuthClient::from(Client::from_framed_transport(t).unwrap());

        // We start a separate task for the client to avoid blocking since
        // we also need to receive the client's request and respond
        let task = tokio::spawn(async move {
            client.handshake().await.unwrap();
            client
                .verify(AuthVerifyKind::Host, "some text".to_string())
                .await
        });

        // Wait for a handshake request and set up our encryption codec
        let request: Request<Auth> = server.read().await.unwrap().unwrap();
        let mut codec = match request.payload {
            Auth::Handshake { public_key, salt } => {
                let handshake = Handshake::default();
                let key = handshake.handshake(public_key, salt).unwrap();
                server
                    .write(Response::new(
                        request.id,
                        Auth::Handshake {
                            public_key: handshake.pk_bytes(),
                            salt: *handshake.salt(),
                        },
                    ))
                    .await
                    .unwrap();
                XChaCha20Poly1305Codec::new(&key)
            }
            _ => panic!("Server received unexpected payload"),
        };

        // Wait for a verify request and send back wrong response
        let request: Request<Auth> = server.read().await.unwrap().unwrap();
        match request.payload {
            Auth::Msg { encrypted_payload } => {
                match decrypt_and_deserialize(&mut codec, &encrypted_payload).unwrap() {
                    AuthRequest::Verify { .. } => {
                        server
                            .write(Response::new(
                                request.id,
                                Auth::Msg {
                                    encrypted_payload: serialize_and_encrypt(
                                        &mut codec,
                                        &AuthResponse::Challenge {
                                            answers: Vec::new(),
                                        },
                                    )
                                    .unwrap(),
                                },
                            ))
                            .await
                            .unwrap();
                    }
                    _ => panic!("Server received wrong request type"),
                }
            }
            _ => panic!("Server received unexpected payload"),
        };

        // Verify that we got an error with the method
        let result = task.await.unwrap();
        assert!(result.is_err(), "Verify succeeded unexpectedly")
    }

    #[tokio::test]
    async fn verify_should_return_valid_bool_received_from_server() {
        let (t, mut server) = FramedTransport::make_test_pair();
        let mut client = AuthClient::from(Client::from_framed_transport(t).unwrap());

        // We start a separate task for the client to avoid blocking since
        // we also need to receive the client's request and respond
        let task = tokio::spawn(async move {
            client.handshake().await.unwrap();
            client
                .verify(AuthVerifyKind::Host, "some text".to_string())
                .await
        });

        // Wait for a handshake request and set up our encryption codec
        let request: Request<Auth> = server.read().await.unwrap().unwrap();
        let mut codec = match request.payload {
            Auth::Handshake { public_key, salt } => {
                let handshake = Handshake::default();
                let key = handshake.handshake(public_key, salt).unwrap();
                server
                    .write(Response::new(
                        request.id,
                        Auth::Handshake {
                            public_key: handshake.pk_bytes(),
                            salt: *handshake.salt(),
                        },
                    ))
                    .await
                    .unwrap();
                XChaCha20Poly1305Codec::new(&key)
            }
            _ => panic!("Server received unexpected payload"),
        };

        // Wait for a challenge request and send back wrong response
        let request: Request<Auth> = server.read().await.unwrap().unwrap();
        match request.payload {
            Auth::Msg { encrypted_payload } => {
                match decrypt_and_deserialize(&mut codec, &encrypted_payload).unwrap() {
                    AuthRequest::Verify { kind, text } => {
                        assert_eq!(kind, AuthVerifyKind::Host);
                        assert_eq!(text, "some text");

                        server
                            .write(Response::new(
                                request.id,
                                Auth::Msg {
                                    encrypted_payload: serialize_and_encrypt(
                                        &mut codec,
                                        &AuthResponse::Verify { valid: true },
                                    )
                                    .unwrap(),
                                },
                            ))
                            .await
                            .unwrap();
                    }
                    _ => panic!("Server received wrong request type"),
                }
            }
            _ => panic!("Server received unexpected payload"),
        };

        // Verify that we got the right results
        let valid = task.await.unwrap().unwrap();
        assert!(valid, "Got verify response, but valid was set incorrectly");
    }

    #[tokio::test]
    async fn info_should_fail_if_handshake_not_finished() {
        let (t, mut server) = FramedTransport::make_test_pair();
        let mut client = AuthClient::from(Client::from_framed_transport(t).unwrap());

        // We start a separate task for the client to avoid blocking since
        // we also need to receive the client's request and respond
        let task = tokio::spawn(async move { client.info("some text".to_string()).await });

        // Wait for a request, failing if we get one as the failure
        // should have prevented sending anything, but we should
        tokio::select! {
            x = TypedAsyncRead::<Request<Auth>>::read(&mut server) => {
                match x {
                    Ok(Some(x)) => panic!("Unexpectedly resolved: {:?}", x),
                    Ok(None) => {},
                    Err(x) => panic!("Unexpectedly failed on server side: {}", x),
                }
            },
            _ = wait_ms(TIMEOUT_MILLIS) => {
                panic!("Should have gotten server closure as part of client exit");
            }
        }

        // Verify that we got an error with the method
        let result = task.await.unwrap();
        assert!(result.is_err(), "Info succeeded unexpectedly")
    }

    #[tokio::test]
    async fn info_should_send_the_server_a_request_but_not_wait_for_a_response() {
        let (t, mut server) = FramedTransport::make_test_pair();
        let mut client = AuthClient::from(Client::from_framed_transport(t).unwrap());

        // We start a separate task for the client to avoid blocking since
        // we also need to receive the client's request and respond
        let task = tokio::spawn(async move {
            client.handshake().await.unwrap();
            client.info("some text".to_string()).await
        });

        // Wait for a handshake request and set up our encryption codec
        let request: Request<Auth> = server.read().await.unwrap().unwrap();
        let mut codec = match request.payload {
            Auth::Handshake { public_key, salt } => {
                let handshake = Handshake::default();
                let key = handshake.handshake(public_key, salt).unwrap();
                server
                    .write(Response::new(
                        request.id,
                        Auth::Handshake {
                            public_key: handshake.pk_bytes(),
                            salt: *handshake.salt(),
                        },
                    ))
                    .await
                    .unwrap();
                XChaCha20Poly1305Codec::new(&key)
            }
            _ => panic!("Server received unexpected payload"),
        };

        // Wait for a request
        let request: Request<Auth> = server.read().await.unwrap().unwrap();
        match request.payload {
            Auth::Msg { encrypted_payload } => {
                match decrypt_and_deserialize(&mut codec, &encrypted_payload).unwrap() {
                    AuthRequest::Info { text } => {
                        assert_eq!(text, "some text");
                    }
                    _ => panic!("Server received wrong request type"),
                }
            }
            _ => panic!("Server received unexpected payload"),
        };

        // Verify that we got the right results
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn error_should_fail_if_handshake_not_finished() {
        let (t, mut server) = FramedTransport::make_test_pair();
        let mut client = AuthClient::from(Client::from_framed_transport(t).unwrap());

        // We start a separate task for the client to avoid blocking since
        // we also need to receive the client's request and respond
        let task = tokio::spawn(async move {
            client
                .error(AuthErrorKind::FailedChallenge, "some text".to_string())
                .await
        });

        // Wait for a request, failing if we get one as the failure
        // should have prevented sending anything, but we should
        tokio::select! {
            x = TypedAsyncRead::<Request<Auth>>::read(&mut server) => {
                match x {
                    Ok(Some(x)) => panic!("Unexpectedly resolved: {:?}", x),
                    Ok(None) => {},
                    Err(x) => panic!("Unexpectedly failed on server side: {}", x),
                }
            },
            _ = wait_ms(TIMEOUT_MILLIS) => {
                panic!("Should have gotten server closure as part of client exit");
            }
        }

        // Verify that we got an error with the method
        let result = task.await.unwrap();
        assert!(result.is_err(), "Error succeeded unexpectedly")
    }

    #[tokio::test]
    async fn error_should_send_the_server_a_request_but_not_wait_for_a_response() {
        let (t, mut server) = FramedTransport::make_test_pair();
        let mut client = AuthClient::from(Client::from_framed_transport(t).unwrap());

        // We start a separate task for the client to avoid blocking since
        // we also need to receive the client's request and respond
        let task = tokio::spawn(async move {
            client.handshake().await.unwrap();
            client
                .error(AuthErrorKind::FailedChallenge, "some text".to_string())
                .await
        });

        // Wait for a handshake request and set up our encryption codec
        let request: Request<Auth> = server.read().await.unwrap().unwrap();
        let mut codec = match request.payload {
            Auth::Handshake { public_key, salt } => {
                let handshake = Handshake::default();
                let key = handshake.handshake(public_key, salt).unwrap();
                server
                    .write(Response::new(
                        request.id,
                        Auth::Handshake {
                            public_key: handshake.pk_bytes(),
                            salt: *handshake.salt(),
                        },
                    ))
                    .await
                    .unwrap();
                XChaCha20Poly1305Codec::new(&key)
            }
            _ => panic!("Server received unexpected payload"),
        };

        // Wait for a request
        let request: Request<Auth> = server.read().await.unwrap().unwrap();
        match request.payload {
            Auth::Msg { encrypted_payload } => {
                match decrypt_and_deserialize(&mut codec, &encrypted_payload).unwrap() {
                    AuthRequest::Error { kind, text } => {
                        assert_eq!(kind, AuthErrorKind::FailedChallenge);
                        assert_eq!(text, "some text");
                    }
                    _ => panic!("Server received wrong request type"),
                }
            }
            _ => panic!("Server received unexpected payload"),
        };

        // Verify that we got the right results
        task.await.unwrap().unwrap();
    }

    async fn wait_ms(ms: u64) {
        use std::time::Duration;
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }

    fn serialize_and_encrypt<T: Serialize>(
        codec: &mut XChaCha20Poly1305Codec,
        payload: &T,
    ) -> io::Result<Vec<u8>> {
        let mut encryped_payload = BytesMut::new();
        let payload = utils::serialize_to_vec(payload)?;
        codec.encode(&payload, &mut encryped_payload)?;
        Ok(encryped_payload.freeze().to_vec())
    }

    fn decrypt_and_deserialize<T: DeserializeOwned>(
        codec: &mut XChaCha20Poly1305Codec,
        payload: &[u8],
    ) -> io::Result<T> {
        let mut payload = BytesMut::from(payload);
        match codec.decode(&mut payload)? {
            Some(payload) => utils::deserialize_from_slice::<T>(&payload),
            None => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Incomplete message received",
            )),
        }
    }
}
