use crate::config::NetworkConfig;
use async_trait::async_trait;
use distant_core::net::client::{Client as NetClient, ReconnectStrategy};
use distant_core::net::common::authentication::msg::*;
use distant_core::net::common::authentication::{
    AuthHandler, AuthMethodHandler, PromptAuthMethodHandler, SingleAuthHandler,
};
use distant_core::net::manager::ManagerClient;
use log::*;
use std::io;
use std::time::Duration;

mod msg;
pub use msg::*;

pub struct Client<T> {
    network: NetworkConfig,
    auth_handler: T,
}

impl Client<()> {
    pub fn new(network: NetworkConfig) -> Self {
        Self {
            network,
            auth_handler: (),
        }
    }
}

impl<T> Client<T> {
    pub fn using_json_auth_handler(self) -> Client<JsonAuthHandler> {
        Client {
            network: self.network,
            auth_handler: JsonAuthHandler::default(),
        }
    }

    pub fn using_prompt_auth_handler(self) -> Client<PromptAuthHandler> {
        todo!();
    }
}

impl<T: AuthHandler + Clone> Client<T> {
    /// Connect to the manager listening on the socket or windows pipe based on
    /// the [`NetworkConfig`] provided to the client earlier. Will return a new instance
    /// of the [`ManagerClient`] upon successful connection
    pub async fn connect(self) -> anyhow::Result<ManagerClient> {
        #[cfg(unix)]
        {
            let mut maybe_client = None;
            let mut error: Option<anyhow::Error> = None;
            for path in self.network.to_unix_socket_path_candidates() {
                match NetClient::unix_socket(path)
                    .auth_handler(self.auth_handler.clone())
                    .reconnect_strategy(ReconnectStrategy::ExponentialBackoff {
                        base: Duration::from_secs(1),
                        factor: 2.0,
                        max_duration: None,
                        max_retries: None,
                        timeout: None,
                    })
                    .connect()
                    .await
                {
                    Ok(client) => {
                        info!("Connected to unix socket @ {:?}", path);
                        maybe_client = Some(client);
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

            Ok(maybe_client.ok_or_else(|| {
                error.unwrap_or_else(|| anyhow::anyhow!("No unix socket candidate available"))
            })?)
        }

        #[cfg(windows)]
        let transport = {
            let mut maybe_client = None;
            let mut error: Option<anyhow::Error> = None;
            for name in self.network.to_windows_pipe_name_candidates() {
                match NetClient::local_windows_pipe(name)
                    .auth_handler(self.auth_handler.clone())
                    .reconnect_strategy(ReconnectStrategy::ExponentialBackoff {
                        base: Duration::from_secs(1),
                        factor: 2.0,
                        max_duration: None,
                        max_retries: None,
                        timeout: None,
                    })
                    .connect()
                    .await
                {
                    Ok(client) => {
                        info!("Connected to named windows pipe @ {:?}", name);
                        maybe_client = Some(client);
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

            Ok(maybe_client.ok_or_else(|| {
                error.unwrap_or_else(|| anyhow::anyhow!("No windows pipe candidate available"))
            })?)
        };
    }
}

/// Implementation of [`AuthHandler`] that communicates over JSON.
#[derive(Clone)]
pub struct JsonAuthHandler {
    tx: MsgSender,
    rx: MsgReceiver,
}

impl JsonAuthHandler {
    pub fn new(tx: MsgSender, rx: MsgReceiver) -> Self {
        Self { tx, rx }
    }
}

impl Default for JsonAuthHandler {
    fn default() -> Self {
        Self::new(MsgSender::from_stdout(), MsgReceiver::from_stdin())
    }
}

#[async_trait]
impl AuthHandler for JsonAuthHandler {
    async fn on_initialization(
        &mut self,
        initialization: Initialization,
    ) -> io::Result<InitializationResponse> {
        self.tx
            .send_blocking(&Authentication::Initialization(initialization))?;
        let response = self.rx.recv_blocking::<AuthenticationResponse>()?;

        match response {
            AuthenticationResponse::Initialization(x) => Ok(x),
            x => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unexpected response: {x:?}"),
            )),
        }
    }

    async fn on_start_method(&mut self, start_method: StartMethod) -> io::Result<()> {
        self.tx
            .send_blocking(&Authentication::StartMethod(start_method))?;
        Ok(())
    }

    async fn on_finished(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[async_trait]
impl AuthMethodHandler for JsonAuthHandler {
    async fn on_challenge(&mut self, challenge: Challenge) -> io::Result<ChallengeResponse> {
        self.tx
            .send_blocking(&Authentication::Challenge(challenge))?;
        let response = self.rx.recv_blocking::<AuthenticationResponse>()?;

        match response {
            AuthenticationResponse::Challenge(x) => Ok(x),
            x => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unexpected response: {x:?}"),
            )),
        }
    }

    async fn on_verification(
        &mut self,
        verification: Verification,
    ) -> io::Result<VerificationResponse> {
        self.tx
            .send_blocking(&Authentication::Verification(verification))?;
        let response = self.rx.recv_blocking::<AuthenticationResponse>()?;

        match response {
            AuthenticationResponse::Verification(x) => Ok(x),
            x => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unexpected response: {x:?}"),
            )),
        }
    }

    async fn on_info(&mut self, info: Info) -> io::Result<()> {
        self.tx.send_blocking(&Authentication::Info(info))?;
        Ok(())
    }

    async fn on_error(&mut self, error: Error) -> io::Result<()> {
        self.tx.send_blocking(&Authentication::Error(error))?;
        Ok(())
    }
}

/// Implementation of [`AuthHandler`] that uses prompts to perform authentication requests and
/// notification of different information.
pub struct PromptAuthHandler(Box<dyn AuthHandler>);

impl PromptAuthHandler {
    pub fn new() -> Self {
        Self(Box::new(SingleAuthHandler::new(
            PromptAuthMethodHandler::new(
                |prompt: &str| {
                    eprintln!("{prompt}");
                    let mut line = String::new();
                    std::io::stdin().read_line(&mut line)?;
                    Ok(line)
                },
                |prompt: &str| rpassword::prompt_password(prompt),
            ),
        )))
    }
}

impl Clone for PromptAuthHandler {
    /// Clones a new copy of the handler.
    ///
    /// ### Note
    ///
    /// This is a hack so we can use this handler elsewhere. Because this handler only has a new
    /// method that creates a new instance, we treat it like a clone and just create an entirely
    /// new prompt auth handler since there is no actual state to clone.
    fn clone(&self) -> Self {
        Self::new()
    }
}

#[async_trait]
impl AuthHandler for PromptAuthHandler {
    async fn on_initialization(
        &mut self,
        initialization: Initialization,
    ) -> io::Result<InitializationResponse> {
        self.0.on_initialization(initialization).await
    }

    async fn on_start_method(&mut self, start_method: StartMethod) -> io::Result<()> {
        self.0.on_start_method(start_method).await
    }

    async fn on_finished(&mut self) -> io::Result<()> {
        self.0.on_finished().await
    }
}

#[async_trait]
impl AuthMethodHandler for PromptAuthHandler {
    async fn on_challenge(&mut self, challenge: Challenge) -> io::Result<ChallengeResponse> {
        self.0.on_challenge(challenge).await
    }

    async fn on_verification(
        &mut self,
        verification: Verification,
    ) -> io::Result<VerificationResponse> {
        self.0.on_verification(verification).await
    }

    async fn on_info(&mut self, info: Info) -> io::Result<()> {
        self.0.on_info(info).await
    }

    async fn on_error(&mut self, error: Error) -> io::Result<()> {
        self.0.on_error(error).await
    }
}
