use std::io;
use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use distant_core::net::auth::msg::*;
use distant_core::net::auth::{AuthHandler, AuthMethodHandler};
use distant_core::net::client::{Client as NetClient, ClientConfig, ReconnectStrategy};
use distant_core::net::manager::{ManagerClient, PROTOCOL_VERSION};
use indicatif::ProgressBar;
use log::*;

use crate::cli::common::ui::Ui;
use crate::cli::common::{MsgReceiver, MsgSender};
use crate::options::{Format, NetworkSettings};

pub struct Client<T> {
    network: NetworkSettings,
    auth_handler: T,
}

impl Client<()> {
    pub fn new(network: NetworkSettings) -> Self {
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
        Client {
            network: self.network,
            auth_handler: PromptAuthHandler::new(),
        }
    }
}

impl<T: AuthHandler + Clone> Client<T> {
    /// Connect to the manager listening on the socket or windows pipe based on
    /// the [`NetworkSettings`] provided to the client earlier. Will return a new instance
    /// of the [`ManagerClient`] upon successful connection
    pub async fn connect(self) -> anyhow::Result<ManagerClient> {
        let client = self.connect_impl().await?;
        client.on_connection_change(|state| debug!("Client is now {state}"));
        Ok(client)
    }

    async fn connect_impl(self) -> anyhow::Result<ManagerClient> {
        #[cfg(unix)]
        {
            let mut maybe_client = None;
            let mut error: Option<anyhow::Error> = None;
            for path in self.network.to_unix_socket_path_candidates() {
                match NetClient::unix_socket(path)
                    .auth_handler(self.auth_handler.clone())
                    .config(ClientConfig {
                        reconnect_strategy: ReconnectStrategy::ExponentialBackoff {
                            base: Duration::from_secs(1),
                            factor: 2.0,
                            max_duration: Some(Duration::from_secs(10)),
                            max_retries: None,
                            timeout: None,
                        },
                        ..Default::default()
                    })
                    .version(PROTOCOL_VERSION)
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
                            .context(format!("Failed to connect to unix socket {path:?}"));
                        if let Some(x) = error {
                            error = Some(x.context(err));
                        } else {
                            error = Some(err);
                        }
                    }
                }
            }

            maybe_client.ok_or_else(|| {
                error.unwrap_or_else(|| anyhow::anyhow!("No unix socket candidate available"))
            })
        }

        #[cfg(windows)]
        {
            let mut maybe_client = None;
            let mut error: Option<anyhow::Error> = None;
            for name in self.network.to_windows_pipe_name_candidates() {
                match NetClient::local_windows_pipe(name)
                    .auth_handler(self.auth_handler.clone())
                    .config(ClientConfig {
                        reconnect_strategy: ReconnectStrategy::ExponentialBackoff {
                            base: Duration::from_secs(1),
                            factor: 2.0,
                            max_duration: Some(Duration::from_secs(10)),
                            max_retries: None,
                            timeout: None,
                        },
                        ..Default::default()
                    })
                    .version(PROTOCOL_VERSION)
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

            maybe_client.ok_or_else(|| {
                error.unwrap_or_else(|| anyhow::anyhow!("No windows pipe candidate available"))
            })
        }
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
        self.tx.send_blocking(&Authentication::Finished)?;
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
/// notification of different information. Optionally holds a [`ProgressBar`] to suspend the
/// spinner while prompting, preventing visual conflicts on stderr.
pub struct PromptAuthHandler {
    pb: Option<ProgressBar>,
}

impl PromptAuthHandler {
    pub fn new() -> Self {
        Self { pb: None }
    }

    pub fn with_progress_bar(pb: Option<ProgressBar>) -> Self {
        Self { pb }
    }
}

impl Clone for PromptAuthHandler {
    fn clone(&self) -> Self {
        Self {
            pb: self.pb.clone(),
        }
    }
}

#[async_trait]
impl AuthHandler for PromptAuthHandler {}

#[async_trait]
impl AuthMethodHandler for PromptAuthHandler {
    async fn on_challenge(&mut self, challenge: Challenge) -> io::Result<ChallengeResponse> {
        let mut answers = Vec::new();
        for question in challenge.questions.iter() {
            let mut lines = question.text.split('\n').collect::<Vec<_>>();
            let line = lines.pop().unwrap();

            let answer = match &self.pb {
                Some(pb) => pb.suspend(|| {
                    for l in &lines {
                        eprintln!("{l}");
                    }
                    rpassword::prompt_password(line).unwrap_or_default()
                }),
                None => {
                    for l in &lines {
                        eprintln!("{l}");
                    }
                    rpassword::prompt_password(line).unwrap_or_default()
                }
            };
            answers.push(answer);
        }
        Ok(ChallengeResponse { answers })
    }

    async fn on_verification(
        &mut self,
        verification: Verification,
    ) -> io::Result<VerificationResponse> {
        match verification.kind {
            VerificationKind::Host => {
                let answer = match &self.pb {
                    Some(pb) => pb.suspend(|| {
                        eprintln!("{}", verification.text);
                        let mut line = String::new();
                        eprint!("Enter [y/N]> ");
                        std::io::stdin().read_line(&mut line).ok();
                        line
                    }),
                    None => {
                        eprintln!("{}", verification.text);
                        let mut line = String::new();
                        eprint!("Enter [y/N]> ");
                        std::io::stdin().read_line(&mut line).ok();
                        line
                    }
                };
                Ok(VerificationResponse {
                    valid: matches!(answer.trim(), "y" | "Y" | "yes" | "YES"),
                })
            }
            x => {
                log::error!("Unsupported verify kind: {x}");
                Ok(VerificationResponse { valid: false })
            }
        }
    }

    async fn on_info(&mut self, info: Info) -> io::Result<()> {
        match &self.pb {
            Some(pb) => pb.suspend(|| eprintln!("{}", info.text)),
            None => eprintln!("{}", info.text),
        }
        Ok(())
    }

    async fn on_error(&mut self, error: Error) -> io::Result<()> {
        match &self.pb {
            Some(pb) => pb.suspend(|| eprintln!("{}: {}", error.kind, error.text)),
            None => eprintln!("{}: {}", error.kind, error.text),
        }
        Ok(())
    }
}

/// Attempt to start the manager daemon by spawning `distant manager listen --daemon`.
/// Returns Ok(()) if the process was spawned successfully, Err otherwise.
fn start_manager_daemon(network: &NetworkSettings) -> anyhow::Result<()> {
    let exe = std::env::current_exe().context("Failed to determine distant executable path")?;

    let mut cmd = std::process::Command::new(exe);
    cmd.args(["manager", "listen", "--daemon"]);

    // Forward custom socket/pipe settings so the new manager listens on the same address
    #[cfg(unix)]
    if let Some(ref socket) = network.unix_socket {
        cmd.args(["--unix-socket", &socket.to_string_lossy()]);
    }
    #[cfg(windows)]
    if let Some(ref pipe) = network.windows_pipe {
        cmd.args(["--windows-pipe", pipe]);
    }

    let status = cmd
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .context("Failed to spawn distant manager")?;

    if status.success() {
        Ok(())
    } else {
        anyhow::bail!(
            "distant manager listen --daemon exited with status {}",
            status
        );
    }
}

/// Connect to the manager, auto-starting it if not already running.
///
/// This is the shared implementation used by both client and manager commands.
/// Provides visual feedback via the `Ui` abstraction (spinners, status messages).
pub async fn connect_to_manager(
    format: Format,
    network: NetworkSettings,
    ui: &Ui,
) -> anyhow::Result<ManagerClient> {
    // First attempt: try connecting directly
    let sp = ui.spinner("Connecting to manager...");
    let first_err = match try_connect(format, &network).await {
        Ok(client) => {
            sp.done("Connected to manager");
            return Ok(client);
        }
        Err(err) => err,
    };

    // Connection failed — try to auto-start the manager
    sp.set_message("Starting manager...");
    ui.warning("Manager not running, starting it...");
    if let Err(err) = start_manager_daemon(&network) {
        warn!("Failed to auto-start manager: {err}");
        sp.fail("Could not start manager");
        return Err(first_err.context(
            "Could not connect to the distant manager, and auto-start failed. \
             Run `distant manager listen --daemon` to start it manually.",
        ));
    }

    // Retry with backoff: 100ms, 200ms, 400ms, 800ms, 500ms = ~2s total
    sp.set_message("Waiting for manager...");
    let delays = [100, 200, 400, 800, 500];
    for delay_ms in delays {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
        match try_connect(format, &network).await {
            Ok(client) => {
                sp.done("Connected to manager");
                return Ok(client);
            }
            Err(_) => continue,
        }
    }

    // Final attempt with the full error context
    match try_connect(format, &network).await {
        Ok(client) => {
            sp.done("Connected to manager");
            Ok(client)
        }
        Err(err) => {
            sp.fail("Failed to connect to manager");
            Err(err.context(
                "Failed to connect to the distant manager after auto-starting it. \
                 Try running `distant manager listen --daemon` manually.",
            ))
        }
    }
}

/// Try to connect to the manager without auto-starting it.
pub async fn try_connect(
    format: Format,
    network: &NetworkSettings,
) -> anyhow::Result<ManagerClient> {
    match format {
        Format::Shell => {
            Client::new(network.clone())
                .using_prompt_auth_handler()
                .connect()
                .await
        }
        Format::Json => {
            Client::new(network.clone())
                .using_json_auth_handler()
                .connect()
                .await
        }
    }
}
