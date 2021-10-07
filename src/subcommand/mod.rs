use crate::opt::{Method, SessionInput, SshConnectionOpts};
use distant_core::{
    LspData, PlainCodec, Session, SessionInfo, SessionInfoFile, XChaCha20Poly1305Codec,
};
use std::{
    future::Future,
    io,
    net::SocketAddr,
    path::{Path, PathBuf},
    pin::Pin,
    time::Duration,
};

pub mod action;
pub mod launch;
pub mod listen;
pub mod lsp;

struct CommandRunner {
    method: Method,
    ssh_connection: SshConnectionOpts,
    session_input: SessionInput,
    session_file: PathBuf,
    session_socket: PathBuf,
    timeout: Duration,
}

impl CommandRunner {
    async fn run<F1, F2, E>(self, start: F1, wrap_err: F2) -> Result<(), E>
    where
        F1: FnOnce(
            Session,
            Duration,
            Option<LspData>,
        ) -> Pin<Box<dyn Future<Output = Result<(), E>>>>,
        F2: Fn(io::Error) -> E + Copy,
        E: std::error::Error,
    {
        let CommandRunner {
            method,
            ssh_connection,
            session_input,
            session_file,
            session_socket,
            timeout,
        } = self;

        let (session, lsp_data) = match method {
            #[cfg(feature = "ssh2")]
            Method::Ssh => {
                use distant_ssh2::{Ssh2Session, Ssh2SessionOpts};
                let SshConnectionOpts { host, port, user } = ssh_connection;

                let mut session = Ssh2Session::connect(
                    host,
                    Ssh2SessionOpts {
                        port: Some(port),
                        user,
                        ..Default::default()
                    },
                )
                .map_err(wrap_err)?;

                session
                    .authenticate(Default::default())
                    .await
                    .map_err(wrap_err)?;

                (session.into_ssh_client_session().map_err(wrap_err)?, None)
            }

            Method::Distant => {
                let params = retrieve_session_params(session_input, session_file, session_socket)
                    .await
                    .map_err(wrap_err)?;
                match params {
                    SessionParams::Tcp {
                        addr,
                        codec,
                        lsp_data,
                    } => {
                        let session = Session::tcp_connect_timeout(addr, codec, timeout)
                            .await
                            .map_err(wrap_err)?;
                        (session, lsp_data)
                    }
                    #[cfg(unix)]
                    SessionParams::Socket { path, codec } => {
                        let session = Session::unix_connect_timeout(path, codec, timeout)
                            .await
                            .map_err(wrap_err)?;
                        (session, None)
                    }
                }
            }
        };

        start(session, timeout, lsp_data).await
    }
}

enum SessionParams {
    Tcp {
        addr: SocketAddr,
        codec: XChaCha20Poly1305Codec,
        lsp_data: Option<LspData>,
    },
    #[cfg(unix)]
    Socket { path: PathBuf, codec: PlainCodec },
}

async fn retrieve_session_params(
    session_input: SessionInput,
    session_file: impl AsRef<Path>,
    session_socket: impl AsRef<Path>,
) -> io::Result<SessionParams> {
    Ok(match session_input {
        SessionInput::Environment => {
            let info = SessionInfo::from_environment()?;
            let addr = info.to_socket_addr().await?;
            let codec = XChaCha20Poly1305Codec::from(info.key);
            SessionParams::Tcp {
                addr,
                codec,
                lsp_data: None,
            }
        }
        SessionInput::File => {
            let info: SessionInfo = SessionInfoFile::load_from(session_file).await?.into();
            let addr = info.to_socket_addr().await?;
            let codec = XChaCha20Poly1305Codec::from(info.key);
            SessionParams::Tcp {
                addr,
                codec,
                lsp_data: None,
            }
        }
        SessionInput::Pipe => {
            let info = SessionInfo::from_stdin()?;
            let addr = info.to_socket_addr().await?;
            let codec = XChaCha20Poly1305Codec::from(info.key);
            SessionParams::Tcp {
                addr,
                codec,
                lsp_data: None,
            }
        }
        SessionInput::Lsp => {
            let mut data =
                LspData::from_buf_reader(&mut io::stdin().lock()).map_err(io::Error::from)?;
            let info = data.take_session_info().map_err(io::Error::from)?;
            let addr = info.to_socket_addr().await?;
            let codec = XChaCha20Poly1305Codec::from(info.key);
            SessionParams::Tcp {
                addr,
                codec,
                lsp_data: Some(data),
            }
        }
        #[cfg(unix)]
        SessionInput::Socket => {
            let path = session_socket.as_ref().to_path_buf();
            let codec = PlainCodec::new();
            SessionParams::Socket { path, codec }
        }
    })
}
