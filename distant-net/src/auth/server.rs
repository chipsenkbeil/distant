use crate::{
    utils, Auth, AuthErrorKind, AuthQuestion, AuthRequest, AuthResponse, AuthVerifyKind, Codec,
    Handshake, Server, ServerCtx, XChaCha20Poly1305Codec,
};
use async_trait::async_trait;
use bytes::BytesMut;
use log::*;
use std::{collections::HashMap, io};
use tokio::sync::RwLock;

/// Type signature for a dynamic on_challenge function
pub type AuthChallengeFn =
    dyn Fn(Vec<AuthQuestion>, HashMap<String, String>) -> Vec<String> + Send + Sync;

/// Type signature for a dynamic on_verify function
pub type AuthVerifyFn = dyn Fn(AuthVerifyKind, String) -> bool + Send + Sync;

/// Type signature for a dynamic on_info function
pub type AuthInfoFn = dyn Fn(String) + Send + Sync;

/// Type signature for a dynamic on_error function
pub type AuthErrorFn = dyn Fn(AuthErrorKind, String) + Send + Sync;

/// Represents an [`AuthServer`] where all handlers are stored on the heap
pub type HeapAuthServer =
    AuthServer<Box<AuthChallengeFn>, Box<AuthVerifyFn>, Box<AuthInfoFn>, Box<AuthErrorFn>>;

/// Server that handles authentication
pub struct AuthServer<ChallengeFn, VerifyFn, InfoFn, ErrorFn>
where
    ChallengeFn: Fn(Vec<AuthQuestion>, HashMap<String, String>) -> Vec<String> + Send + Sync,
    VerifyFn: Fn(AuthVerifyKind, String) -> bool + Send + Sync,
    InfoFn: Fn(String) + Send + Sync,
    ErrorFn: Fn(AuthErrorKind, String) + Send + Sync,
{
    pub on_challenge: ChallengeFn,
    pub on_verify: VerifyFn,
    pub on_info: InfoFn,
    pub on_error: ErrorFn,
}

#[async_trait]
impl<ChallengeFn, VerifyFn, InfoFn, ErrorFn> Server
    for AuthServer<ChallengeFn, VerifyFn, InfoFn, ErrorFn>
where
    ChallengeFn: Fn(Vec<AuthQuestion>, HashMap<String, String>) -> Vec<String> + Send + Sync,
    VerifyFn: Fn(AuthVerifyKind, String) -> bool + Send + Sync,
    InfoFn: Fn(String) + Send + Sync,
    ErrorFn: Fn(AuthErrorKind, String) + Send + Sync,
{
    type Request = Auth;
    type Response = Auth;
    type LocalData = RwLock<Option<XChaCha20Poly1305Codec>>;

    async fn on_request(&self, ctx: ServerCtx<Self::Request, Self::Response, Self::LocalData>) {
        let reply = ctx.reply.clone();

        match ctx.request.payload {
            Auth::Handshake { public_key, salt } => {
                let handshake = Handshake::default();
                match handshake.handshake(public_key, salt) {
                    Ok(key) => {
                        ctx.local_data
                            .write()
                            .await
                            .replace(XChaCha20Poly1305Codec::new(&key));

                        if let Err(x) = reply
                            .send(Auth::Handshake {
                                public_key: handshake.pk_bytes(),
                                salt: *handshake.salt(),
                            })
                            .await
                        {
                            error!("[Conn {}] {}", ctx.connection_id, x);
                        }
                    }
                    Err(x) => {
                        error!("[Conn {}] {}", ctx.connection_id, x);
                        return;
                    }
                }
            }
            Auth::Msg {
                ref encrypted_payload,
            } => {
                // Attempt to decrypt the message so we can understand what to do
                let request = match ctx.local_data.write().await.as_mut() {
                    Some(codec) => {
                        let mut payload = BytesMut::from(encrypted_payload.as_slice());
                        match codec.decode(&mut payload) {
                            Ok(Some(payload)) => {
                                utils::deserialize_from_slice::<AuthRequest>(&payload)
                            }
                            Ok(None) => Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "Incomplete message received",
                            )),
                            Err(x) => Err(x),
                        }
                    }
                    None => Err(io::Error::new(
                        io::ErrorKind::Other,
                        "Handshake must be performed first (server decrypt message)",
                    )),
                };

                let response = match request {
                    Ok(request) => match request {
                        AuthRequest::Challenge { questions, extra } => {
                            let answers = (self.on_challenge)(questions, extra);
                            AuthResponse::Challenge { answers }
                        }
                        AuthRequest::Verify { kind, text } => {
                            let valid = (self.on_verify)(kind, text);
                            AuthResponse::Verify { valid }
                        }
                        AuthRequest::Info { text } => {
                            (self.on_info)(text);
                            return;
                        }
                        AuthRequest::Error { kind, text } => {
                            (self.on_error)(kind, text);
                            return;
                        }
                    },
                    Err(x) => {
                        error!("[Conn {}] {}", ctx.connection_id, x);
                        return;
                    }
                };

                // Serialize and encrypt the message before sending it back
                let encrypted_payload = match ctx.local_data.write().await.as_mut() {
                    Some(codec) => {
                        let mut encrypted_payload = BytesMut::new();

                        // Convert the response into bytes for us to send back
                        match utils::serialize_to_vec(&response) {
                            Ok(bytes) => match codec.encode(&bytes, &mut encrypted_payload) {
                                Ok(_) => Ok(encrypted_payload.freeze().to_vec()),
                                Err(x) => Err(x),
                            },
                            Err(x) => Err(x),
                        }
                    }
                    None => Err(io::Error::new(
                        io::ErrorKind::Other,
                        "Handshake must be performed first (server encrypt messaage)",
                    )),
                };

                match encrypted_payload {
                    Ok(encrypted_payload) => {
                        if let Err(x) = reply.send(Auth::Msg { encrypted_payload }).await {
                            error!("[Conn {}] {}", ctx.connection_id, x);
                            return;
                        }
                    }
                    Err(x) => {
                        error!("[Conn {}] {}", ctx.connection_id, x);
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        IntoSplit, MpscListener, MpscTransport, Request, Response, ServerExt, ServerRef,
        TypedAsyncRead, TypedAsyncWrite,
    };
    use tokio::sync::mpsc;

    const TIMEOUT_MILLIS: u64 = 100;

    #[tokio::test]
    async fn should_not_reply_if_receive_encrypted_msg_without_handshake_first() {
        let (mut t, _) = spawn_auth_server(
            /* on_challenge */ |_, _| Vec::new(),
            /* on_verify    */ |_, _| false,
            /* on_info      */ |_| {},
            /* on_error     */ |_, _| {},
        )
        .await
        .expect("Failed to spawn server");

        // Send an encrypted message before establishing a handshake
        t.write(Request::new(Auth::Msg {
            encrypted_payload: Vec::new(),
        }))
        .await
        .expect("Failed to send request to server");

        // Wait for a response, failing if we get one
        tokio::select! {
            x = t.read() => panic!("Unexpectedly resolved: {:?}", x),
            _ = wait_ms(TIMEOUT_MILLIS) => {}
        }
    }

    #[tokio::test]
    async fn should_reply_to_handshake_request_with_new_public_key_and_salt() {
        let (mut t, _) = spawn_auth_server(
            /* on_challenge */ |_, _| Vec::new(),
            /* on_verify    */ |_, _| false,
            /* on_info      */ |_| {},
            /* on_error     */ |_, _| {},
        )
        .await
        .expect("Failed to spawn server");

        // Send a handshake
        let handshake = Handshake::default();
        t.write(Request::new(Auth::Handshake {
            public_key: handshake.pk_bytes(),
            salt: *handshake.salt(),
        }))
        .await
        .expect("Failed to send request to server");

        // Wait for a handshake response
        tokio::select! {
            x = t.read() => {
                let response = x.expect("Request failed").expect("Response missing");
                match response.payload {
                    Auth::Handshake { .. } => {},
                    Auth::Msg { .. } => panic!("Received unexpected encryped message during handshake"),
                }
            }
            _ = wait_ms(TIMEOUT_MILLIS) => panic!("Ran out of time waiting on response"),
        }
    }

    #[tokio::test]
    async fn should_not_reply_if_receive_invalid_encrypted_msg() {
        let (mut t, _) = spawn_auth_server(
            /* on_challenge */ |_, _| Vec::new(),
            /* on_verify    */ |_, _| false,
            /* on_info      */ |_| {},
            /* on_error     */ |_, _| {},
        )
        .await
        .expect("Failed to spawn server");

        // Send a handshake
        let handshake = Handshake::default();
        t.write(Request::new(Auth::Handshake {
            public_key: handshake.pk_bytes(),
            salt: *handshake.salt(),
        }))
        .await
        .expect("Failed to send request to server");

        // Complete handshake
        let key = match t.read().await.unwrap().unwrap().payload {
            Auth::Handshake { public_key, salt } => handshake.handshake(public_key, salt).unwrap(),
            Auth::Msg { .. } => panic!("Received unexpected encryped message during handshake"),
        };

        // Send a bad chunk of data
        let _codec = XChaCha20Poly1305Codec::new(&key);
        t.write(Request::new(Auth::Msg {
            encrypted_payload: vec![1, 2, 3, 4],
        }))
        .await
        .unwrap();

        // Wait for a response, failing if we get one
        tokio::select! {
            x = t.read() => panic!("Unexpectedly resolved: {:?}", x),
            _ = wait_ms(TIMEOUT_MILLIS) => {}
        }
    }

    #[tokio::test]
    async fn should_invoke_appropriate_function_when_receive_challenge_request_and_reply() {
        let (tx, mut rx) = mpsc::channel(1);
        let (mut t, _) = spawn_auth_server(
            /* on_challenge */
            move |questions, extra| {
                tx.try_send((questions, extra)).unwrap();
                vec!["answer1".to_string(), "answer2".to_string()]
            },
            /* on_verify    */ |_, _| false,
            /* on_info      */ |_| {},
            /* on_error     */ |_, _| {},
        )
        .await
        .expect("Failed to spawn server");

        // Send a handshake
        let handshake = Handshake::default();
        t.write(Request::new(Auth::Handshake {
            public_key: handshake.pk_bytes(),
            salt: *handshake.salt(),
        }))
        .await
        .expect("Failed to send request to server");

        // Complete handshake
        let key = match t.read().await.unwrap().unwrap().payload {
            Auth::Handshake { public_key, salt } => handshake.handshake(public_key, salt).unwrap(),
            Auth::Msg { .. } => panic!("Received unexpected encryped message during handshake"),
        };

        // Send an error request
        let mut codec = XChaCha20Poly1305Codec::new(&key);
        t.write(Request::new(Auth::Msg {
            encrypted_payload: serialize_and_encrypt(
                &mut codec,
                &AuthRequest::Challenge {
                    questions: vec![
                        AuthQuestion::new("question1".to_string()),
                        AuthQuestion {
                            text: "question2".to_string(),
                            extra: vec![("key".to_string(), "value".to_string())]
                                .into_iter()
                                .collect(),
                        },
                    ],
                    extra: vec![("hello".to_string(), "world".to_string())]
                        .into_iter()
                        .collect(),
                },
            )
            .unwrap(),
        }))
        .await
        .unwrap();

        // Verify that the handler was triggered
        let (questions, extra) = rx.recv().await.expect("Channel closed unexpectedly");
        assert_eq!(
            questions,
            vec![
                AuthQuestion::new("question1".to_string()),
                AuthQuestion {
                    text: "question2".to_string(),
                    extra: vec![("key".to_string(), "value".to_string())]
                        .into_iter()
                        .collect(),
                }
            ]
        );
        assert_eq!(
            extra,
            vec![("hello".to_string(), "world".to_string())]
                .into_iter()
                .collect()
        );

        // Wait for a response and verify that it matches what we expect
        tokio::select! {
            x = t.read() => {
                let response = x.expect("Request failed").expect("Response missing");
                match response.payload {
                    Auth::Handshake { .. } => panic!("Received unexpected handshake"),
                    Auth::Msg { encrypted_payload } => {
                        match decrypt_and_deserialize(&mut codec, &encrypted_payload).unwrap() {
                            AuthResponse::Challenge { answers } =>
                                assert_eq!(
                                    answers,
                                    vec!["answer1".to_string(), "answer2".to_string()]
                                ),
                            _ => panic!("Got wrong response for verify"),
                        }
                    },
                }
            }
            _ = wait_ms(TIMEOUT_MILLIS) => {}
        }
    }

    #[tokio::test]
    async fn should_invoke_appropriate_function_when_receive_verify_request_and_reply() {
        let (tx, mut rx) = mpsc::channel(1);
        let (mut t, _) = spawn_auth_server(
            /* on_challenge */ |_, _| Vec::new(),
            /* on_verify    */
            move |kind, text| {
                tx.try_send((kind, text)).unwrap();
                true
            },
            /* on_info      */ |_| {},
            /* on_error     */ |_, _| {},
        )
        .await
        .expect("Failed to spawn server");

        // Send a handshake
        let handshake = Handshake::default();
        t.write(Request::new(Auth::Handshake {
            public_key: handshake.pk_bytes(),
            salt: *handshake.salt(),
        }))
        .await
        .expect("Failed to send request to server");

        // Complete handshake
        let key = match t.read().await.unwrap().unwrap().payload {
            Auth::Handshake { public_key, salt } => handshake.handshake(public_key, salt).unwrap(),
            Auth::Msg { .. } => panic!("Received unexpected encryped message during handshake"),
        };

        // Send an error request
        let mut codec = XChaCha20Poly1305Codec::new(&key);
        t.write(Request::new(Auth::Msg {
            encrypted_payload: serialize_and_encrypt(
                &mut codec,
                &AuthRequest::Verify {
                    kind: AuthVerifyKind::Host,
                    text: "some text".to_string(),
                },
            )
            .unwrap(),
        }))
        .await
        .unwrap();

        // Verify that the handler was triggered
        let (kind, text) = rx.recv().await.expect("Channel closed unexpectedly");
        assert_eq!(kind, AuthVerifyKind::Host);
        assert_eq!(text, "some text");

        // Wait for a response and verify that it matches what we expect
        tokio::select! {
            x = t.read() => {
                let response = x.expect("Request failed").expect("Response missing");
                match response.payload {
                    Auth::Handshake { .. } => panic!("Received unexpected handshake"),
                    Auth::Msg { encrypted_payload } => {
                        match decrypt_and_deserialize(&mut codec, &encrypted_payload).unwrap() {
                            AuthResponse::Verify { valid } =>
                                assert!(valid, "Got verify, but valid was wrong"),
                            _ => panic!("Got wrong response for verify"),
                        }
                    },
                }
            }
            _ = wait_ms(TIMEOUT_MILLIS) => {}
        }
    }

    #[tokio::test]
    async fn should_invoke_appropriate_function_when_receive_info_request() {
        let (tx, mut rx) = mpsc::channel(1);
        let (mut t, _) = spawn_auth_server(
            /* on_challenge */ |_, _| Vec::new(),
            /* on_verify    */ |_, _| false,
            /* on_info      */
            move |text| {
                tx.try_send(text).unwrap();
            },
            /* on_error     */ |_, _| {},
        )
        .await
        .expect("Failed to spawn server");

        // Send a handshake
        let handshake = Handshake::default();
        t.write(Request::new(Auth::Handshake {
            public_key: handshake.pk_bytes(),
            salt: *handshake.salt(),
        }))
        .await
        .expect("Failed to send request to server");

        // Complete handshake
        let key = match t.read().await.unwrap().unwrap().payload {
            Auth::Handshake { public_key, salt } => handshake.handshake(public_key, salt).unwrap(),
            Auth::Msg { .. } => panic!("Received unexpected encryped message during handshake"),
        };

        // Send an error request
        let mut codec = XChaCha20Poly1305Codec::new(&key);
        t.write(Request::new(Auth::Msg {
            encrypted_payload: serialize_and_encrypt(
                &mut codec,
                &AuthRequest::Info {
                    text: "some text".to_string(),
                },
            )
            .unwrap(),
        }))
        .await
        .unwrap();

        // Verify that the handler was triggered
        let text = rx.recv().await.expect("Channel closed unexpectedly");
        assert_eq!(text, "some text");

        // Wait for a response, failing if we get one
        tokio::select! {
            x = t.read() => panic!("Unexpectedly resolved: {:?}", x),
            _ = wait_ms(TIMEOUT_MILLIS) => {}
        }
    }

    #[tokio::test]
    async fn should_invoke_appropriate_function_when_receive_error_request() {
        let (tx, mut rx) = mpsc::channel(1);
        let (mut t, _) = spawn_auth_server(
            /* on_challenge */ |_, _| Vec::new(),
            /* on_verify    */ |_, _| false,
            /* on_info      */ |_| {},
            /* on_error     */
            move |kind, text| {
                tx.try_send((kind, text)).unwrap();
            },
        )
        .await
        .expect("Failed to spawn server");

        // Send a handshake
        let handshake = Handshake::default();
        t.write(Request::new(Auth::Handshake {
            public_key: handshake.pk_bytes(),
            salt: *handshake.salt(),
        }))
        .await
        .expect("Failed to send request to server");

        // Complete handshake
        let key = match t.read().await.unwrap().unwrap().payload {
            Auth::Handshake { public_key, salt } => handshake.handshake(public_key, salt).unwrap(),
            Auth::Msg { .. } => panic!("Received unexpected encryped message during handshake"),
        };

        // Send an error request
        let mut codec = XChaCha20Poly1305Codec::new(&key);
        t.write(Request::new(Auth::Msg {
            encrypted_payload: serialize_and_encrypt(
                &mut codec,
                &AuthRequest::Error {
                    kind: AuthErrorKind::FailedChallenge,
                    text: "some text".to_string(),
                },
            )
            .unwrap(),
        }))
        .await
        .unwrap();

        // Verify that the handler was triggered
        let (kind, text) = rx.recv().await.expect("Channel closed unexpectedly");
        assert_eq!(kind, AuthErrorKind::FailedChallenge);
        assert_eq!(text, "some text");

        // Wait for a response, failing if we get one
        tokio::select! {
            x = t.read() => panic!("Unexpectedly resolved: {:?}", x),
            _ = wait_ms(TIMEOUT_MILLIS) => {}
        }
    }

    async fn wait_ms(ms: u64) {
        use std::time::Duration;
        tokio::time::sleep(Duration::from_millis(ms)).await;
    }

    fn serialize_and_encrypt(
        codec: &mut XChaCha20Poly1305Codec,
        payload: &AuthRequest,
    ) -> io::Result<Vec<u8>> {
        let mut encryped_payload = BytesMut::new();
        let payload = utils::serialize_to_vec(payload)?;
        codec.encode(&payload, &mut encryped_payload)?;
        Ok(encryped_payload.freeze().to_vec())
    }

    fn decrypt_and_deserialize(
        codec: &mut XChaCha20Poly1305Codec,
        payload: &[u8],
    ) -> io::Result<AuthResponse> {
        let mut payload = BytesMut::from(payload);
        match codec.decode(&mut payload)? {
            Some(payload) => utils::deserialize_from_slice::<AuthResponse>(&payload),
            None => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Incomplete message received",
            )),
        }
    }

    async fn spawn_auth_server<ChallengeFn, VerifyFn, InfoFn, ErrorFn>(
        on_challenge: ChallengeFn,
        on_verify: VerifyFn,
        on_info: InfoFn,
        on_error: ErrorFn,
    ) -> io::Result<(
        MpscTransport<Request<Auth>, Response<Auth>>,
        Box<dyn ServerRef>,
    )>
    where
        ChallengeFn:
            Fn(Vec<AuthQuestion>, HashMap<String, String>) -> Vec<String> + Send + Sync + 'static,
        VerifyFn: Fn(AuthVerifyKind, String) -> bool + Send + Sync + 'static,
        InfoFn: Fn(String) + Send + Sync + 'static,
        ErrorFn: Fn(AuthErrorKind, String) + Send + Sync + 'static,
    {
        let server = AuthServer {
            on_challenge,
            on_verify,
            on_info,
            on_error,
        };

        // Create a test listener where we will forward a connection
        let (tx, listener) = MpscListener::channel(100);

        // Make bounded transport pair and send off one of them to act as our connection
        let (transport, connection) = MpscTransport::<Request<Auth>, Response<Auth>>::pair(100);
        tx.send(connection.into_split())
            .await
            .expect("Failed to feed listener a connection");

        let server = server.start(listener)?;
        Ok((transport, server))
    }
}
