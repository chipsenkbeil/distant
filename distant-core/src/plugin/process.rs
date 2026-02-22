use std::future::Future;
use std::io;
use std::pin::Pin;
use std::process::Stdio;
use std::time::Duration;

use log::*;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use crate::auth::msg::*;
use crate::auth::Authenticator;
use crate::net::client::{ClientConfig, UntypedClient};
use crate::net::common::{Destination, FramedTransport, InmemoryTransport, Map};

use super::Plugin;

/// Adapter that wraps an external binary as a Plugin.
///
/// The binary communicates via a JSON-lines protocol over stdin/stdout. Two subcommands
/// are supported:
///
/// - `<binary> launch <destination> [--key=value ...]` — spawns a short-lived process that
///   performs a launch operation and prints the resulting destination.
/// - `<binary> connect <destination> [--key=value ...]` — spawns a long-lived process that
///   acts as a bidirectional distant API proxy over stdin/stdout.
///
/// Authentication is relayed as JSON-lines during the setup phase of both subcommands.
/// See `PLUGINS.md` at the repository root for the full external binary specification.
pub struct ProcessPlugin {
    /// Human-readable name for logging and error messages.
    pub name: String,
    /// Path to the external binary.
    pub path: std::path::PathBuf,
    /// URI schemes this plugin handles. Defaults to `[name]` if `None`.
    pub schemes: Option<Vec<String>>,
}

/// Setup-phase timeout: how long we wait for the binary to complete auth + emit
/// its ready/destination message before killing it.
const SETUP_TIMEOUT: Duration = Duration::from_secs(120);

// --- JSON-lines protocol messages ---

#[derive(Deserialize)]
#[serde(untagged)]
enum SetupMessage {
    AuthChallenge {
        auth_challenge: AuthChallengeMsg,
    },
    Ready {
        #[allow(dead_code)]
        ready: bool,
    },
    Destination {
        destination: String,
    },
    Error {
        error: PluginError,
    },
}

#[derive(Deserialize)]
struct AuthChallengeMsg {
    questions: Vec<AuthQuestionMsg>,
    #[serde(default)]
    options: std::collections::HashMap<String, String>,
}

#[derive(Deserialize)]
struct AuthQuestionMsg {
    text: String,
    #[serde(default)]
    label: String,
    #[serde(default)]
    options: std::collections::HashMap<String, String>,
}

#[derive(Serialize)]
struct AuthResponseMsg {
    auth_response: AuthResponseInner,
}

#[derive(Serialize)]
struct AuthResponseInner {
    answers: Vec<String>,
}

#[derive(Deserialize)]
struct PluginError {
    #[serde(default = "default_error_kind")]
    kind: String,
    description: String,
}

fn default_error_kind() -> String {
    "other".to_string()
}

fn map_to_args(options: &Map) -> Vec<String> {
    let mut args = Vec::new();
    for (key, value) in options.iter() {
        args.push(format!("--{key}={value}"));
    }
    args
}

/// Relay authentication challenges from the child process to the authenticator,
/// reading JSON-lines from `reader` and writing responses to `writer`.
/// Returns the first non-auth message encountered (Ready, Destination, or Error).
async fn relay_auth(
    reader: &mut BufReader<tokio::process::ChildStdout>,
    writer: &mut tokio::process::ChildStdin,
    authenticator: &mut dyn Authenticator,
) -> io::Result<SetupMessage> {
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "plugin process closed stdout during setup",
            ));
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: SetupMessage = serde_json::from_str(trimmed).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("invalid JSON from plugin: {e}: {trimmed}"),
            )
        })?;
        match msg {
            SetupMessage::AuthChallenge { auth_challenge } => {
                let questions = auth_challenge
                    .questions
                    .into_iter()
                    .map(|q| Question {
                        label: q.label,
                        text: q.text,
                        options: q.options,
                    })
                    .collect();
                let response = authenticator
                    .challenge(Challenge {
                        questions,
                        options: auth_challenge.options,
                    })
                    .await?;
                let resp_msg = AuthResponseMsg {
                    auth_response: AuthResponseInner {
                        answers: response.answers,
                    },
                };
                let mut json = serde_json::to_string(&resp_msg).map_err(io::Error::other)?;
                json.push('\n');
                writer.write_all(json.as_bytes()).await?;
                writer.flush().await?;
            }
            other => return Ok(other),
        }
    }
}

fn plugin_error_to_io(err: PluginError) -> io::Error {
    let kind = match err.kind.as_str() {
        "not_found" => io::ErrorKind::NotFound,
        "permission_denied" => io::ErrorKind::PermissionDenied,
        "connection_refused" => io::ErrorKind::ConnectionRefused,
        "unsupported" => io::ErrorKind::Unsupported,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, err.description)
}

impl Plugin for ProcessPlugin {
    fn name(&self) -> &str {
        &self.name
    }

    fn schemes(&self) -> Vec<String> {
        match &self.schemes {
            Some(schemes) => schemes.clone(),
            None => vec![self.name.clone()],
        }
    }

    fn connect<'a>(
        &'a self,
        destination: &'a Destination,
        options: &'a Map,
        authenticator: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<UntypedClient>> + Send + 'a>> {
        Box::pin(async move {
            debug!(
                "[{}] Spawning connect process: {} connect {}",
                self.name,
                self.path.display(),
                destination
            );

            let mut child = Command::new(&self.path)
                .arg("connect")
                .arg(destination.to_string())
                .args(map_to_args(options))
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| {
                    io::Error::new(
                        e.kind(),
                        format!("failed to spawn plugin '{}': {e}", self.name),
                    )
                })?;

            let mut stdout = BufReader::new(child.stdout.take().unwrap());
            let mut stdin = child.stdin.take().unwrap();

            // Setup phase with timeout: relay auth challenges until we get {"ready": true}
            let setup_result = tokio::time::timeout(
                SETUP_TIMEOUT,
                relay_auth(&mut stdout, &mut stdin, authenticator),
            )
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "plugin '{}' timed out during connect setup ({}s)",
                        self.name,
                        SETUP_TIMEOUT.as_secs()
                    ),
                )
            })??;

            match setup_result {
                SetupMessage::Ready { .. } => {
                    debug!("[{}] Plugin connect ready, bridging stdio", self.name);
                }
                SetupMessage::Error { error } => return Err(plugin_error_to_io(error)),
                SetupMessage::Destination { .. } => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "plugin '{}' sent destination during connect (expected ready)",
                            self.name
                        ),
                    ));
                }
                SetupMessage::AuthChallenge { .. } => unreachable!(),
            }

            // After ready, the child's stdin/stdout become a bidirectional JSON-lines transport.
            // Bridge it through InmemoryTransport channels so we can construct an UntypedClient.
            let (tx_to_client, rx_from_bridge) = tokio::sync::mpsc::channel::<Vec<u8>>(64);
            let (tx_to_bridge, rx_for_child) = tokio::sync::mpsc::channel::<Vec<u8>>(64);

            let transport = InmemoryTransport::new(tx_to_bridge, rx_from_bridge);

            // Task: read from child stdout and forward to client via channel
            let plugin_name = self.name.clone();
            tokio::spawn(async move {
                let mut stdout = stdout;
                let mut line = String::new();
                loop {
                    line.clear();
                    match stdout.read_line(&mut line).await {
                        Ok(0) => break,
                        Ok(_) => {
                            if tx_to_client.send(line.as_bytes().to_vec()).await.is_err() {
                                break;
                            }
                        }
                        Err(e) => {
                            debug!("[{plugin_name}] stdout read error: {e}");
                            break;
                        }
                    }
                }
            });

            // Task: read from client channel and write to child stdin
            let plugin_name2 = self.name.clone();
            tokio::spawn(async move {
                let mut stdin = stdin;
                let mut rx = rx_for_child;
                while let Some(data) = rx.recv().await {
                    if let Err(e) = stdin.write_all(&data).await {
                        debug!("[{plugin_name2}] stdin write error: {e}");
                        break;
                    }
                    if let Err(e) = stdin.flush().await {
                        debug!("[{plugin_name2}] stdin flush error: {e}");
                        break;
                    }
                }
            });

            let framed = FramedTransport::plain(transport);
            Ok(UntypedClient::spawn_inmemory(
                framed,
                ClientConfig::default(),
            ))
        })
    }

    fn launch<'a>(
        &'a self,
        destination: &'a Destination,
        options: &'a Map,
        authenticator: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<Destination>> + Send + 'a>> {
        Box::pin(async move {
            debug!(
                "[{}] Spawning launch process: {} launch {}",
                self.name,
                self.path.display(),
                destination
            );

            let mut child = Command::new(&self.path)
                .arg("launch")
                .arg(destination.to_string())
                .args(map_to_args(options))
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .map_err(|e| {
                    io::Error::new(
                        e.kind(),
                        format!("failed to spawn plugin '{}': {e}", self.name),
                    )
                })?;

            let mut stdout = BufReader::new(child.stdout.take().unwrap());
            let mut stdin = child.stdin.take().unwrap();

            // Setup phase with timeout: relay auth, then expect a destination or error
            let setup_result = tokio::time::timeout(
                SETUP_TIMEOUT,
                relay_auth(&mut stdout, &mut stdin, authenticator),
            )
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "plugin '{}' timed out during launch setup ({}s)",
                        self.name,
                        SETUP_TIMEOUT.as_secs()
                    ),
                )
            })??;

            match setup_result {
                SetupMessage::Destination { destination: dest } => {
                    let parsed: Destination = dest.parse().map_err(|e| {
                        io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("plugin '{}' returned invalid destination: {e}", self.name),
                        )
                    })?;
                    debug!("[{}] Launch returned destination: {parsed}", self.name);
                    Ok(parsed)
                }
                SetupMessage::Error { error } => Err(plugin_error_to_io(error)),
                SetupMessage::Ready { .. } => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "plugin '{}' sent ready during launch (expected destination)",
                        self.name
                    ),
                )),
                SetupMessage::AuthChallenge { .. } => unreachable!(),
            }
        })
    }
}
