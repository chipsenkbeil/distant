use std::future::Future;
use std::io;
use std::pin::Pin;
use std::time::Duration;

use anyhow::Context;
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

impl AuthHandler for JsonAuthHandler {
    fn on_initialization<'a>(
        &'a mut self,
        initialization: Initialization,
    ) -> Pin<Box<dyn Future<Output = io::Result<InitializationResponse>> + Send + 'a>> {
        Box::pin(async move {
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
        })
    }

    fn on_start_method<'a>(
        &'a mut self,
        start_method: StartMethod,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.tx
                .send_blocking(&Authentication::StartMethod(start_method))?;
            Ok(())
        })
    }

    fn on_finished<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.tx.send_blocking(&Authentication::Finished)?;
            Ok(())
        })
    }
}

impl AuthMethodHandler for JsonAuthHandler {
    fn on_challenge<'a>(
        &'a mut self,
        challenge: Challenge,
    ) -> Pin<Box<dyn Future<Output = io::Result<ChallengeResponse>> + Send + 'a>> {
        Box::pin(async move {
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
        })
    }

    fn on_verification<'a>(
        &'a mut self,
        verification: Verification,
    ) -> Pin<Box<dyn Future<Output = io::Result<VerificationResponse>> + Send + 'a>> {
        Box::pin(async move {
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
        })
    }

    fn on_info<'a>(
        &'a mut self,
        info: Info,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.tx.send_blocking(&Authentication::Info(info))?;
            Ok(())
        })
    }

    fn on_error<'a>(
        &'a mut self,
        error: Error,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.tx.send_blocking(&Authentication::Error(error))?;
            Ok(())
        })
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

impl AuthHandler for PromptAuthHandler {}

impl AuthMethodHandler for PromptAuthHandler {
    fn on_challenge<'a>(
        &'a mut self,
        challenge: Challenge,
    ) -> Pin<Box<dyn Future<Output = io::Result<ChallengeResponse>> + Send + 'a>> {
        Box::pin(async move {
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
        })
    }

    fn on_verification<'a>(
        &'a mut self,
        verification: Verification,
    ) -> Pin<Box<dyn Future<Output = io::Result<VerificationResponse>> + Send + 'a>> {
        Box::pin(async move {
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
        })
    }

    fn on_info<'a>(
        &'a mut self,
        info: Info,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            match &self.pb {
                Some(pb) => pb.suspend(|| eprintln!("{}", info.text)),
                None => eprintln!("{}", info.text),
            }
            Ok(())
        })
    }

    fn on_error<'a>(
        &'a mut self,
        error: Error,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            match &self.pb {
                Some(pb) => pb.suspend(|| eprintln!("{}: {}", error.kind, error.text)),
                None => eprintln!("{}: {}", error.kind, error.text),
            }
            Ok(())
        })
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use distant_core::net::auth::msg::*;
    use test_log::test;

    use super::*;

    // -------------------------------------------------------
    // Client::new — construction
    // -------------------------------------------------------
    #[test]
    fn client_new_creates_with_unit_auth_handler() {
        let network = NetworkSettings::default();
        let client = Client::new(network.clone());
        assert_eq!(client.network, network);
    }

    // -------------------------------------------------------
    // Client::using_json_auth_handler — swaps handler type
    // -------------------------------------------------------
    #[test]
    fn client_using_json_auth_handler_preserves_network() {
        let network = NetworkSettings {
            unix_socket: Some(std::path::PathBuf::from("/tmp/test.sock")),
            windows_pipe: None,
        };
        let client = Client::new(network.clone()).using_json_auth_handler();
        assert_eq!(client.network, network);
    }

    // -------------------------------------------------------
    // Client::using_prompt_auth_handler — swaps handler type
    // -------------------------------------------------------
    #[test]
    fn client_using_prompt_auth_handler_preserves_network() {
        let network = NetworkSettings {
            unix_socket: None,
            windows_pipe: Some("test-pipe".to_string()),
        };
        let client = Client::new(network.clone()).using_prompt_auth_handler();
        assert_eq!(client.network, network);
    }

    // -------------------------------------------------------
    // JsonAuthHandler — construction
    // -------------------------------------------------------
    #[test]
    fn json_auth_handler_new_does_not_panic() {
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let output_clone = Arc::clone(&output);

        let tx = MsgSender::from(move |data: &[u8]| -> io::Result<()> {
            output_clone.lock().unwrap().extend_from_slice(data);
            Ok(())
        });
        let rx = MsgReceiver::from(move |_input: &mut String| -> io::Result<()> { Ok(()) });

        let _handler = JsonAuthHandler::new(tx, rx);
    }

    // -------------------------------------------------------
    // JsonAuthHandler — on_start_method sends StartMethod
    // -------------------------------------------------------
    #[test_log::test(tokio::test)]
    async fn json_auth_handler_on_start_method_sends_json() {
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let output_clone = Arc::clone(&output);

        let tx = MsgSender::from(move |data: &[u8]| -> io::Result<()> {
            output_clone.lock().unwrap().extend_from_slice(data);
            Ok(())
        });
        let rx = MsgReceiver::from(move |_input: &mut String| -> io::Result<()> { Ok(()) });

        let mut handler = JsonAuthHandler::new(tx, rx);

        use distant_core::net::auth::AuthHandler;
        let start_method = StartMethod {
            method: "password".to_string(),
        };
        handler.on_start_method(start_method).await.unwrap();

        let written = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(!written.is_empty());
        // Should contain the method kind
        assert!(written.contains("password"));
    }

    // -------------------------------------------------------
    // JsonAuthHandler — on_finished sends Finished
    // -------------------------------------------------------
    #[test_log::test(tokio::test)]
    async fn json_auth_handler_on_finished_sends_json() {
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let output_clone = Arc::clone(&output);

        let tx = MsgSender::from(move |data: &[u8]| -> io::Result<()> {
            output_clone.lock().unwrap().extend_from_slice(data);
            Ok(())
        });
        let rx = MsgReceiver::from(move |_input: &mut String| -> io::Result<()> { Ok(()) });

        let mut handler = JsonAuthHandler::new(tx, rx);

        use distant_core::net::auth::AuthHandler;
        handler.on_finished().await.unwrap();

        let written = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(!written.is_empty());
    }

    // -------------------------------------------------------
    // JsonAuthHandler — on_info sends info
    // -------------------------------------------------------
    #[test_log::test(tokio::test)]
    async fn json_auth_handler_on_info_sends_json() {
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let output_clone = Arc::clone(&output);

        let tx = MsgSender::from(move |data: &[u8]| -> io::Result<()> {
            output_clone.lock().unwrap().extend_from_slice(data);
            Ok(())
        });
        let rx = MsgReceiver::from(move |_input: &mut String| -> io::Result<()> { Ok(()) });

        let mut handler = JsonAuthHandler::new(tx, rx);

        use distant_core::net::auth::AuthMethodHandler;
        let info = Info {
            text: "test info".to_string(),
        };
        handler.on_info(info).await.unwrap();

        let written = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(written.contains("test info"));
    }

    // -------------------------------------------------------
    // JsonAuthHandler — on_error sends error
    // -------------------------------------------------------
    #[test_log::test(tokio::test)]
    async fn json_auth_handler_on_error_sends_json() {
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let output_clone = Arc::clone(&output);

        let tx = MsgSender::from(move |data: &[u8]| -> io::Result<()> {
            output_clone.lock().unwrap().extend_from_slice(data);
            Ok(())
        });
        let rx = MsgReceiver::from(move |_input: &mut String| -> io::Result<()> { Ok(()) });

        let mut handler = JsonAuthHandler::new(tx, rx);

        use distant_core::net::auth::AuthMethodHandler;
        let error = Error {
            kind: ErrorKind::Fatal,
            text: "something failed".to_string(),
        };
        handler.on_error(error).await.unwrap();

        let written = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert!(written.contains("something failed"));
    }

    // -------------------------------------------------------
    // JsonAuthHandler — on_challenge round-trip
    // -------------------------------------------------------
    #[test_log::test(tokio::test)]
    async fn json_auth_handler_on_challenge_round_trip() {
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let output_clone = Arc::clone(&output);

        let tx = MsgSender::from(move |data: &[u8]| -> io::Result<()> {
            output_clone.lock().unwrap().extend_from_slice(data);
            Ok(())
        });

        // Simulate a response that sends back a ChallengeResponse
        let response = AuthenticationResponse::Challenge(ChallengeResponse {
            answers: vec!["my-password".to_string()],
        });
        let response_json = format!("{}\n", serde_json::to_string(&response).unwrap());
        let sent = Arc::new(Mutex::new(false));
        let sent_clone = Arc::clone(&sent);

        let rx = MsgReceiver::from(move |input: &mut String| -> io::Result<()> {
            let mut guard = sent_clone.lock().unwrap();
            if !*guard {
                *guard = true;
                input.push_str(&response_json);
                Ok(())
            } else {
                Err(io::Error::new(io::ErrorKind::UnexpectedEof, "no more data"))
            }
        });

        let mut handler = JsonAuthHandler::new(tx, rx);

        use distant_core::net::auth::AuthMethodHandler;
        let challenge = Challenge {
            questions: vec![Question {
                label: "password".to_string(),
                text: "Enter password: ".to_string(),
                options: Default::default(),
            }],
            options: Default::default(),
        };
        let result = handler.on_challenge(challenge).await.unwrap();
        assert_eq!(result.answers, ["my-password"]);
    }

    // -------------------------------------------------------
    // JsonAuthHandler — on_challenge wrong response type errors
    // -------------------------------------------------------
    #[test_log::test(tokio::test)]
    async fn json_auth_handler_on_challenge_wrong_response_type() {
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let output_clone = Arc::clone(&output);

        let tx = MsgSender::from(move |data: &[u8]| -> io::Result<()> {
            output_clone.lock().unwrap().extend_from_slice(data);
            Ok(())
        });

        // Send a Verification response instead of Challenge
        let response = AuthenticationResponse::Verification(VerificationResponse { valid: true });
        let response_json = format!("{}\n", serde_json::to_string(&response).unwrap());
        let sent = Arc::new(Mutex::new(false));
        let sent_clone = Arc::clone(&sent);

        let rx = MsgReceiver::from(move |input: &mut String| -> io::Result<()> {
            let mut guard = sent_clone.lock().unwrap();
            if !*guard {
                *guard = true;
                input.push_str(&response_json);
                Ok(())
            } else {
                Err(io::Error::new(io::ErrorKind::UnexpectedEof, "no more data"))
            }
        });

        let mut handler = JsonAuthHandler::new(tx, rx);

        use distant_core::net::auth::AuthMethodHandler;
        let challenge = Challenge {
            questions: vec![],
            options: Default::default(),
        };
        let result = handler.on_challenge(challenge).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    // -------------------------------------------------------
    // JsonAuthHandler — on_verification round-trip
    // -------------------------------------------------------
    #[test_log::test(tokio::test)]
    async fn json_auth_handler_on_verification_round_trip() {
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let output_clone = Arc::clone(&output);

        let tx = MsgSender::from(move |data: &[u8]| -> io::Result<()> {
            output_clone.lock().unwrap().extend_from_slice(data);
            Ok(())
        });

        let response = AuthenticationResponse::Verification(VerificationResponse { valid: true });
        let response_json = format!("{}\n", serde_json::to_string(&response).unwrap());
        let sent = Arc::new(Mutex::new(false));
        let sent_clone = Arc::clone(&sent);

        let rx = MsgReceiver::from(move |input: &mut String| -> io::Result<()> {
            let mut guard = sent_clone.lock().unwrap();
            if !*guard {
                *guard = true;
                input.push_str(&response_json);
                Ok(())
            } else {
                Err(io::Error::new(io::ErrorKind::UnexpectedEof, "no more data"))
            }
        });

        let mut handler = JsonAuthHandler::new(tx, rx);

        use distant_core::net::auth::AuthMethodHandler;
        let verification = Verification {
            kind: VerificationKind::Host,
            text: "fingerprint: abc123".to_string(),
        };
        let result = handler.on_verification(verification).await.unwrap();
        assert!(result.valid);
    }

    // -------------------------------------------------------
    // JsonAuthHandler — on_verification wrong response type
    // -------------------------------------------------------
    #[test_log::test(tokio::test)]
    async fn json_auth_handler_on_verification_wrong_response_type() {
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let output_clone = Arc::clone(&output);

        let tx = MsgSender::from(move |data: &[u8]| -> io::Result<()> {
            output_clone.lock().unwrap().extend_from_slice(data);
            Ok(())
        });

        // Send Challenge response instead of Verification
        let response = AuthenticationResponse::Challenge(ChallengeResponse { answers: vec![] });
        let response_json = format!("{}\n", serde_json::to_string(&response).unwrap());
        let sent = Arc::new(Mutex::new(false));
        let sent_clone = Arc::clone(&sent);

        let rx = MsgReceiver::from(move |input: &mut String| -> io::Result<()> {
            let mut guard = sent_clone.lock().unwrap();
            if !*guard {
                *guard = true;
                input.push_str(&response_json);
                Ok(())
            } else {
                Err(io::Error::new(io::ErrorKind::UnexpectedEof, "no more data"))
            }
        });

        let mut handler = JsonAuthHandler::new(tx, rx);

        use distant_core::net::auth::AuthMethodHandler;
        let verification = Verification {
            kind: VerificationKind::Host,
            text: "test".to_string(),
        };
        let result = handler.on_verification(verification).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
    }

    // -------------------------------------------------------
    // PromptAuthHandler — construction
    // -------------------------------------------------------
    #[test]
    fn prompt_auth_handler_new_does_not_panic() {
        let _handler = PromptAuthHandler::new();
    }

    #[test]
    fn prompt_auth_handler_with_progress_bar_none() {
        let handler = PromptAuthHandler::with_progress_bar(None);
        assert!(handler.pb.is_none());
    }

    #[test]
    fn prompt_auth_handler_with_progress_bar_some() {
        let pb = ProgressBar::new(100);
        let handler = PromptAuthHandler::with_progress_bar(Some(pb));
        assert!(handler.pb.is_some());
    }

    // -------------------------------------------------------
    // PromptAuthHandler — clone
    // -------------------------------------------------------
    #[test]
    fn prompt_auth_handler_clone() {
        let handler = PromptAuthHandler::new();
        let cloned = handler.clone();
        assert!(cloned.pb.is_none());
    }

    // -------------------------------------------------------
    // JsonAuthHandler — clone
    // -------------------------------------------------------
    #[test]
    fn json_auth_handler_clone_shares_sender_and_receiver() {
        let output = Arc::new(Mutex::new(Vec::<u8>::new()));
        let output_clone = Arc::clone(&output);

        let tx = MsgSender::from(move |data: &[u8]| -> io::Result<()> {
            output_clone.lock().unwrap().extend_from_slice(data);
            Ok(())
        });
        let rx = MsgReceiver::from(move |_input: &mut String| -> io::Result<()> { Ok(()) });

        let handler = JsonAuthHandler::new(tx, rx);
        let _cloned = handler.clone();
        // Just verify clone doesn't panic
    }
}
