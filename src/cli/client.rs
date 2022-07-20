use crate::config::NetworkConfig;
use distant_core::{
    net::{AuthRequest, AuthResponse, FramedTransport, PlainCodec},
    DistantManagerClient, DistantManagerClientConfig,
};
use log::*;
use std::io;

mod msg;
pub use msg::*;

pub struct Client {
    config: DistantManagerClientConfig,
    network: NetworkConfig,
}

impl Client {
    pub fn new(network: NetworkConfig) -> Self {
        let config = DistantManagerClientConfig::with_prompts(
            |prompt| rpassword::prompt_password(prompt),
            |prompt| {
                use std::io::Write;
                eprint!("{}", prompt);
                std::io::stderr().lock().flush()?;

                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                Ok(answer)
            },
        );
        Self { config, network }
    }

    /// Configure client to talk over stdin and stdout using messages
    pub fn using_msg_stdin_stdout(self) -> Self {
        self.using_msg(MsgSender::from_stdout(), MsgReceiver::from_stdin())
    }

    /// Configure client to use a pair of msg sender and receiver
    pub fn using_msg(mut self, tx: MsgSender, rx: MsgReceiver) -> Self {
        self.config = DistantManagerClientConfig {
            on_challenge: {
                let tx = tx.clone();
                let rx = rx.clone();
                Box::new(move |questions, extra| {
                    let question_cnt = questions.len();

                    if let Err(x) = tx.send_blocking(&AuthRequest::Challenge { questions, extra }) {
                        error!("{}", x);
                        return (0..question_cnt)
                            .into_iter()
                            .map(|_| "".to_string())
                            .collect();
                    }

                    match rx.recv_blocking() {
                        Ok(AuthResponse::Challenge { answers }) => answers,
                        Ok(x) => {
                            error!("Invalid response received: {:?}", x);
                            (0..question_cnt)
                                .into_iter()
                                .map(|_| "".to_string())
                                .collect()
                        }
                        Err(x) => {
                            error!("{}", x);
                            (0..question_cnt)
                                .into_iter()
                                .map(|_| "".to_string())
                                .collect()
                        }
                    }
                })
            },
            on_info: {
                let tx = tx.clone();
                Box::new(move |text| {
                    let _ = tx.send_blocking(&AuthRequest::Info { text });
                })
            },
            on_verify: {
                let tx = tx.clone();
                Box::new(move |kind, text| {
                    if let Err(x) = tx.send_blocking(&AuthRequest::Verify { kind, text }) {
                        error!("{}", x);
                        return false;
                    }

                    match rx.recv_blocking() {
                        Ok(AuthResponse::Verify { valid }) => valid,
                        Ok(x) => {
                            error!("Invalid response received: {:?}", x);
                            false
                        }
                        Err(x) => {
                            error!("{}", x);
                            false
                        }
                    }
                })
            },
            on_error: {
                Box::new(move |kind, text| {
                    let _ = tx.send_blocking(&AuthRequest::Error { kind, text });
                })
            },
        };
        self
    }

    /// Connect to the manager listening on the socket or windows pipe based on
    /// the [`NetworkConfig`] provided to the client earlier. Will return a new instance
    /// of the [`DistantManagerClient`] upon successful connection
    pub async fn connect(self) -> io::Result<DistantManagerClient> {
        #[cfg(unix)]
        let transport = {
            use distant_core::net::UnixSocketTransport;
            // TODO: Try multiple, collect errors, and then fail
            for path in self.network.to_unix_socket_path_candidates() {
                let transport = UnixSocketTransport::connect(path).await?;
                FramedTransport::new(, PlainCodec)
            }

        };

        #[cfg(windows)]
        let transport = {
            use distant_core::net::WindowsPipeTransport;
            FramedTransport::new(
                WindowsPipeTransport::connect_local(self.network.windows_pipe_name_or_default())
                    .await?,
                PlainCodec,
            )
        };

        DistantManagerClient::new(self.config, transport)
    }
}
