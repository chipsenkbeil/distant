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

#[cfg(test)]
mod tests {
    //! Tests for ProcessPlugin: construction, schemes, map_to_args, plugin_error_to_io mapping,
    //! SetupMessage deserialization (JSON-lines protocol), AuthResponseMsg serialization,
    //! and connect/launch error paths with nonexistent binaries.

    use std::path::PathBuf;

    use test_log::test;

    use super::*;

    // -----------------------------------------------------------------------
    // ProcessPlugin construction and Plugin trait methods
    // -----------------------------------------------------------------------

    fn make_plugin(name: &str, path: &str, schemes: Option<Vec<String>>) -> ProcessPlugin {
        ProcessPlugin {
            name: name.to_string(),
            path: PathBuf::from(path),
            schemes,
        }
    }

    #[test]
    fn name_returns_the_configured_name() {
        let plugin = make_plugin("ssh", "/usr/bin/distant-ssh", None);
        assert_eq!(plugin.name(), "ssh");
    }

    #[test]
    fn schemes_defaults_to_name_when_none() {
        let plugin = make_plugin("docker", "/usr/bin/distant-docker", None);
        assert_eq!(plugin.schemes(), vec!["docker".to_string()]);
    }

    #[test]
    fn schemes_returns_configured_schemes_when_some() {
        let plugin = make_plugin(
            "docker",
            "/usr/bin/distant-docker",
            Some(vec!["docker".into(), "docker-compose".into()]),
        );
        assert_eq!(
            plugin.schemes(),
            vec!["docker".to_string(), "docker-compose".to_string()]
        );
    }

    #[test]
    fn schemes_returns_empty_vec_when_configured_as_empty() {
        let plugin = make_plugin("test", "/bin/test", Some(vec![]));
        let schemes = plugin.schemes();
        assert!(schemes.is_empty());
    }

    // -----------------------------------------------------------------------
    // map_to_args helper
    // -----------------------------------------------------------------------

    #[test]
    fn map_to_args_returns_empty_for_empty_map() {
        let map = Map::new();
        let args = map_to_args(&map);
        assert!(args.is_empty());
    }

    #[test]
    fn map_to_args_produces_flag_format() {
        let mut map = Map::new();
        map.insert("port".to_string(), "22".to_string());
        let args = map_to_args(&map);
        assert_eq!(args.len(), 1);
        assert_eq!(args[0], "--port=22");
    }

    #[test]
    fn map_to_args_handles_multiple_entries() {
        let mut map = Map::new();
        map.insert("host".to_string(), "localhost".to_string());
        map.insert("port".to_string(), "8080".to_string());
        let args = map_to_args(&map);
        assert_eq!(args.len(), 2);
        // HashMap order is nondeterministic, so check that both are present
        assert!(args.contains(&"--host=localhost".to_string()));
        assert!(args.contains(&"--port=8080".to_string()));
    }

    #[test]
    fn map_to_args_handles_empty_value() {
        let mut map = Map::new();
        map.insert("flag".to_string(), String::new());
        let args = map_to_args(&map);
        assert_eq!(args, vec!["--flag="]);
    }

    // -----------------------------------------------------------------------
    // plugin_error_to_io mapping
    // -----------------------------------------------------------------------

    #[test]
    fn plugin_error_to_io_maps_not_found() {
        let err = PluginError {
            kind: "not_found".to_string(),
            description: "file missing".to_string(),
        };
        let io_err = plugin_error_to_io(err);
        assert_eq!(io_err.kind(), io::ErrorKind::NotFound);
        assert_eq!(io_err.to_string(), "file missing");
    }

    #[test]
    fn plugin_error_to_io_maps_permission_denied() {
        let err = PluginError {
            kind: "permission_denied".to_string(),
            description: "access denied".to_string(),
        };
        let io_err = plugin_error_to_io(err);
        assert_eq!(io_err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn plugin_error_to_io_maps_connection_refused() {
        let err = PluginError {
            kind: "connection_refused".to_string(),
            description: "refused".to_string(),
        };
        let io_err = plugin_error_to_io(err);
        assert_eq!(io_err.kind(), io::ErrorKind::ConnectionRefused);
    }

    #[test]
    fn plugin_error_to_io_maps_unsupported() {
        let err = PluginError {
            kind: "unsupported".to_string(),
            description: "not supported".to_string(),
        };
        let io_err = plugin_error_to_io(err);
        assert_eq!(io_err.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn plugin_error_to_io_maps_unknown_kind_to_other() {
        let err = PluginError {
            kind: "something_else".to_string(),
            description: "mysterious error".to_string(),
        };
        let io_err = plugin_error_to_io(err);
        assert_eq!(io_err.kind(), io::ErrorKind::Other);
        assert_eq!(io_err.to_string(), "mysterious error");
    }

    // -----------------------------------------------------------------------
    // default_error_kind
    // -----------------------------------------------------------------------

    #[test]
    fn default_error_kind_returns_other() {
        assert_eq!(default_error_kind(), "other");
    }

    // -----------------------------------------------------------------------
    // SetupMessage deserialization (JSON-lines protocol)
    // -----------------------------------------------------------------------

    #[test]
    fn setup_message_deserializes_ready() {
        let json = r#"{"ready": true}"#;
        let msg: SetupMessage = serde_json::from_str(json).expect("parse ready");
        assert!(matches!(msg, SetupMessage::Ready { ready: true }));
    }

    #[test]
    fn setup_message_deserializes_destination() {
        let json = r#"{"destination": "ssh://user@host:22"}"#;
        let msg: SetupMessage = serde_json::from_str(json).expect("parse destination");
        match msg {
            SetupMessage::Destination { destination } => {
                assert_eq!(destination, "ssh://user@host:22");
            }
            _ => panic!("expected Destination variant"),
        }
    }

    #[test]
    fn setup_message_deserializes_error_with_kind() {
        let json = r#"{"error": {"kind": "not_found", "description": "binary not found"}}"#;
        let msg: SetupMessage = serde_json::from_str(json).expect("parse error");
        match msg {
            SetupMessage::Error { error } => {
                assert_eq!(error.kind, "not_found");
                assert_eq!(error.description, "binary not found");
            }
            _ => panic!("expected Error variant"),
        }
    }

    #[test]
    fn setup_message_deserializes_error_with_default_kind() {
        let json = r#"{"error": {"description": "something went wrong"}}"#;
        let msg: SetupMessage = serde_json::from_str(json).expect("parse error");
        match msg {
            SetupMessage::Error { error } => {
                assert_eq!(error.kind, "other");
                assert_eq!(error.description, "something went wrong");
            }
            _ => panic!("expected Error variant"),
        }
    }

    #[test]
    fn setup_message_deserializes_auth_challenge() {
        let json = r#"{
            "auth_challenge": {
                "questions": [
                    {"text": "Password:", "label": "password"}
                ],
                "options": {"method": "keyboard-interactive"}
            }
        }"#;
        let msg: SetupMessage = serde_json::from_str(json).expect("parse auth_challenge");
        match msg {
            SetupMessage::AuthChallenge { auth_challenge } => {
                assert_eq!(auth_challenge.questions.len(), 1);
                assert_eq!(auth_challenge.questions[0].text, "Password:");
                assert_eq!(auth_challenge.questions[0].label, "password");
                assert_eq!(
                    auth_challenge.options.get("method").map(String::as_str),
                    Some("keyboard-interactive")
                );
            }
            _ => panic!("expected AuthChallenge variant"),
        }
    }

    #[test]
    fn setup_message_deserializes_auth_challenge_with_defaults() {
        // Minimal auth_challenge: only text is required on questions, label and options default
        let json = r#"{"auth_challenge": {"questions": [{"text": "Enter code:"}]}}"#;
        let msg: SetupMessage = serde_json::from_str(json).expect("parse auth_challenge");
        match msg {
            SetupMessage::AuthChallenge { auth_challenge } => {
                assert_eq!(auth_challenge.questions.len(), 1);
                assert_eq!(auth_challenge.questions[0].text, "Enter code:");
                assert_eq!(auth_challenge.questions[0].label, "");
                assert!(auth_challenge.questions[0].options.is_empty());
                assert!(auth_challenge.options.is_empty());
            }
            _ => panic!("expected AuthChallenge variant"),
        }
    }

    // -----------------------------------------------------------------------
    // AuthResponseMsg serialization
    // -----------------------------------------------------------------------

    #[test]
    fn auth_response_msg_serializes_correctly() {
        let msg = AuthResponseMsg {
            auth_response: AuthResponseInner {
                answers: vec!["my_password".to_string()],
            },
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse back");
        assert_eq!(
            parsed["auth_response"]["answers"][0].as_str(),
            Some("my_password")
        );
    }

    #[test]
    fn auth_response_msg_serializes_multiple_answers() {
        let msg = AuthResponseMsg {
            auth_response: AuthResponseInner {
                answers: vec!["answer1".to_string(), "answer2".to_string()],
            },
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse back");
        let answers = parsed["auth_response"]["answers"]
            .as_array()
            .expect("answers should be array");
        assert_eq!(answers.len(), 2);
        assert_eq!(answers[0].as_str(), Some("answer1"));
        assert_eq!(answers[1].as_str(), Some("answer2"));
    }

    #[test]
    fn auth_response_msg_serializes_empty_answers() {
        let msg = AuthResponseMsg {
            auth_response: AuthResponseInner { answers: vec![] },
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse back");
        let answers = parsed["auth_response"]["answers"]
            .as_array()
            .expect("answers should be array");
        assert!(answers.is_empty());
    }

    // -----------------------------------------------------------------------
    // connect/launch with nonexistent binary (error path)
    //
    // These tests verify that when the external binary does not exist,
    // connect() and launch() return an appropriate io::Error. This
    // exercises the spawn error handling without needing a real plugin
    // binary.
    // -----------------------------------------------------------------------

    use crate::auth::TestAuthenticator;

    #[test(tokio::test)]
    async fn connect_fails_with_nonexistent_binary() {
        let plugin = make_plugin("fake", "/nonexistent/path/to/plugin", None);
        let dest: Destination = "ssh://localhost".parse().expect("parse destination");
        let options = Map::new();

        let mut auth = TestAuthenticator::default();
        let result = plugin.connect(&dest, &options, &mut auth).await;
        assert!(result.is_err(), "expected error for nonexistent binary");
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(
            err.to_string().contains("fake"),
            "error message should mention plugin name, got: {}",
            err
        );
    }

    #[test(tokio::test)]
    async fn launch_fails_with_nonexistent_binary() {
        let plugin = make_plugin("fake", "/nonexistent/path/to/plugin", None);
        let dest: Destination = "ssh://localhost".parse().expect("parse destination");
        let options = Map::new();

        let mut auth = TestAuthenticator::default();
        let result = plugin.launch(&dest, &options, &mut auth).await;
        assert!(result.is_err(), "expected error for nonexistent binary");
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
        assert!(
            err.to_string().contains("fake"),
            "error message should mention plugin name, got: {}",
            err
        );
    }
}
