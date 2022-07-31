use crate::config::NetworkConfig;
use anyhow::Context;
use distant_core::{
    net::{AuthRequest, AuthResponse, FramedTransport, PlainCodec},
    DistantManagerClient, DistantManagerClientConfig,
};
use log::*;

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
    pub async fn connect(self) -> anyhow::Result<DistantManagerClient> {
        #[cfg(unix)]
        let transport = {
            use distant_core::net::UnixSocketTransport;
            let mut maybe_transport = None;
            let mut error: Option<anyhow::Error> = None;
            for path in self.network.to_unix_socket_path_candidates() {
                match UnixSocketTransport::connect(path).await {
                    Ok(transport) => {
                        info!("Connected to unix socket @ {:?}", path);
                        maybe_transport = Some(FramedTransport::new(transport, PlainCodec));
                        break;
                    }
                    Err(x) => {
                        let err = anyhow::Error::new(x)
                            .context(format!("Failed to connect to unix socket {:?}", path));
                        if let Some(x) = error {
                            error = Some(x.context(err));
                        } else {
                            error = Some(err);
                        }
                    }
                }
            }

            maybe_transport.ok_or_else(|| {
                error.unwrap_or_else(|| anyhow::anyhow!("No unix socket candidate available"))
            })?
        };

        #[cfg(windows)]
        let transport = {
            use distant_core::net::WindowsPipeTransport;
            let mut maybe_transport = None;
            let mut error: Option<anyhow::Error> = None;
            for name in self.network.to_windows_pipe_name_candidates() {
                match WindowsPipeTransport::connect_local(name).await {
                    Ok(transport) => {
                        info!("Connected to named windows socket @ {:?}", name);
                        maybe_transport = Some(FramedTransport::new(transport, PlainCodec));
                        break;
                    }
                    Err(x) => {
                        let err = anyhow::Error::new(x)
                            .context(format!("Failed to connect to windows pipe {:?}", name));
                        if let Some(x) = error {
                            error = Some(x.context(err));
                        } else {
                            error = Some(err);
                        }
                    }
                }
            }

            maybe_transport.ok_or_else(|| {
                error.unwrap_or_else(|| anyhow::anyhow!("No windows pipe candidate available"))
            })?
        };

        DistantManagerClient::new(self.config, transport)
            .context("Failed to create client for manager")
    }
}
