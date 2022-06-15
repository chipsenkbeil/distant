use crate::{
    utils, Answers, Auth, AuthErrorKind, AuthExtra, AuthRequest, AuthResponse, AuthVerifyKind,
    Codec, Handshake, Questions, Server, ServerCtx, XChaCha20Poly1305Codec,
};
use async_trait::async_trait;
use bytes::BytesMut;
use log::*;
use std::io;

/// Server that handles authentication
pub struct AuthServer<ChallengeFn, VerifyFn, InfoFn, ErrorFn>
where
    ChallengeFn: Fn(Questions, AuthExtra) -> Answers + Send + Sync,
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
    ChallengeFn: Fn(Questions, AuthExtra) -> Answers + Send + Sync,
    VerifyFn: Fn(AuthVerifyKind, String) -> bool + Send + Sync,
    InfoFn: Fn(String) + Send + Sync,
    ErrorFn: Fn(AuthErrorKind, String) + Send + Sync,
{
    type Request = Auth;
    type Response = Auth;
    type GlobalData = ();
    type LocalData = Option<XChaCha20Poly1305Codec>;

    async fn on_request(
        &self,
        ctx: ServerCtx<Self::Request, Self::Response, Self::GlobalData, Self::LocalData>,
    ) {
        let reply = ctx.reply.clone();

        match ctx.request.payload {
            Auth::Handshake { public_key, salt } => {
                let handshake = Handshake::default();
                match handshake.handshake(public_key, salt) {
                    Ok(key) => {
                        ctx.with_mut_local_data(move |data| {
                            data.replace(XChaCha20Poly1305Codec::new(&key))
                        })
                        .await;

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
            Auth::Msg(ref msg) => {
                // Attempt to decrypt the message so we can understand what to do
                let request = ctx.with_mut_local_data(move |codec| match codec {
                    Some(codec) => {
                        let mut msg = BytesMut::from(msg.as_slice());
                        match codec.decode(&mut msg) {
                            Ok(Some(decrypted_msg)) => {
                                utils::deserialize_from_slice::<AuthRequest>(&decrypted_msg)
                                    .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))
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
                        "Handshake must be performed first",
                    )),
                });

                let response = match request.await {
                    Some(Ok(request)) => match request {
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
                    Some(Err(x)) => {
                        error!("[Conn {}] {}", ctx.connection_id, x);
                        return;
                    }
                    None => {
                        error!(
                            "[Conn {}] Key unavailable for decryption",
                            ctx.connection_id
                        );
                        return;
                    }
                };

                // Serialize and encrypt the message before sending it back
                let msg = ctx.with_mut_local_data(move |codec| match codec {
                    Some(codec) => {
                        let mut encrypted_msg = BytesMut::new();

                        // Convert the response into bytes for us to send back
                        let bytes = match utils::serialize_to_vec(&response) {
                            Ok(x) => x,
                            Err(x) => return Err(x),
                        };

                        match codec.encode(&bytes, &mut encrypted_msg) {
                            Ok(_) => Ok(encrypted_msg.freeze().to_vec()),
                            Err(x) => Err(x),
                        }
                    }
                    None => Err(io::Error::new(
                        io::ErrorKind::Other,
                        "Handshake must be performed first",
                    )),
                });

                match msg.await {
                    Some(Ok(msg)) => {
                        if let Err(x) = reply.send(Auth::Msg(msg)).await {
                            error!("[Conn {}] {}", ctx.connection_id, x);
                            return;
                        }
                    }
                    Some(Err(x)) => {
                        error!("[Conn {}] {}", ctx.connection_id, x);
                        return;
                    }
                    None => {
                        error!(
                            "[Conn {}] Key unavailable for decryption",
                            ctx.connection_id
                        );
                        return;
                    }
                }
            }
        }
    }
}
