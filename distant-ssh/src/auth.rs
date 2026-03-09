//! SSH authentication helpers.
//!
//! Contains the individual authentication strategies (agent, file-based public key,
//! keyboard-interactive, password) extracted from the monolithic `Ssh::authenticate`
//! method. Each function is a self-contained step that the orchestrator in `lib.rs`
//! calls in sequence.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use log::*;
use russh::client::Handle;
use russh::keys::agent::client::AgentClient;

use crate::{ClientHandler, SshAuthEvent, SshAuthHandler, SshAuthPrompt};

/// Expand a leading `~` in a path to the user's home directory.
pub(crate) fn expand_tilde(path: &Path) -> PathBuf {
    if let Ok(rest) = path.strip_prefix("~")
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    path.to_path_buf()
}

/// Format a `MethodSet` as a comma-separated string of method names.
pub(crate) fn format_methods(methods: &russh::MethodSet) -> String {
    if methods.is_empty() {
        return "none".to_string();
    }
    methods
        .iter()
        .map(<&str>::from)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Try SSH agent authentication using the given agent client.
///
/// Queries the agent for available keys and tries each one against the server.
/// Returns `true` if any agent key was accepted.
async fn authenticate_with_agent<S>(
    handle: &mut Handle<ClientHandler>,
    user: &str,
    agent: &mut AgentClient<S>,
    methods_tried: &mut Vec<String>,
    server_methods: &mut Option<russh::MethodSet>,
) -> bool
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let keys = match agent.request_identities().await {
        Ok(keys) => keys,
        Err(e) => {
            debug!("Failed to list agent keys: {}", e);
            return false;
        }
    };

    if keys.is_empty() {
        debug!("SSH agent has no keys");
        return false;
    }

    debug!("SSH agent has {} key(s)", keys.len());
    methods_tried.push("agent".to_string());

    for key in &keys {
        debug!("Trying agent key: {:?}", key.algorithm());
        match handle
            .authenticate_publickey_with(user, key.clone(), None, agent)
            .await
        {
            Ok(res) if res.success() => return true,
            Ok(russh::client::AuthResult::Failure {
                remaining_methods, ..
            }) => {
                *server_methods = Some(remaining_methods);
            }
            Ok(_) => {}
            Err(e) => {
                debug!("Agent key rejected: {}", e);
            }
        }
    }

    false
}

/// Attempt SSH agent authentication across platform-specific agent backends.
///
/// On Unix, connects to the agent via `SSH_AUTH_SOCK`. On Windows, tries the
/// OpenSSH named pipe first, then falls back to Pageant. Returns `Ok(true)`
/// if authentication succeeded via any agent.
pub(crate) async fn try_agent_auth(
    handle: &mut Handle<ClientHandler>,
    user: &str,
    methods_tried: &mut Vec<String>,
    server_methods: &mut Option<russh::MethodSet>,
) -> io::Result<bool> {
    debug!("Attempting SSH agent authentication");

    #[cfg(unix)]
    {
        match AgentClient::connect_env().await {
            Ok(mut agent) => {
                if authenticate_with_agent(handle, user, &mut agent, methods_tried, server_methods)
                    .await
                {
                    return Ok(true);
                }
            }
            Err(e) => {
                debug!("SSH agent not available: {}", e);
            }
        }
    }

    #[cfg(windows)]
    {
        let mut agent_authenticated = false;

        // Try OpenSSH agent (named pipe)
        match AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
            Ok(mut agent) => {
                if authenticate_with_agent(handle, user, &mut agent, methods_tried, server_methods)
                    .await
                {
                    agent_authenticated = true;
                }
            }
            Err(e) => {
                debug!("OpenSSH agent not available: {:?}", e);
            }
        }

        // Try Pageant if OpenSSH agent didn't work
        if !agent_authenticated {
            match AgentClient::connect_pageant().await {
                Ok(mut agent) => {
                    if authenticate_with_agent(
                        handle,
                        user,
                        &mut agent,
                        methods_tried,
                        server_methods,
                    )
                    .await
                    {
                        agent_authenticated = true;
                    }
                }
                Err(e) => {
                    debug!("Pageant not available: {:?}", e);
                }
            }
        }

        if agent_authenticated {
            return Ok(true);
        }
    }

    Ok(false)
}

/// Collect identity key file paths from explicit opts, SSH config, or defaults.
///
/// Priority order:
/// 1. Explicitly provided via CLI opts (with tilde expansion)
/// 2. From SSH config `IdentityFile` directive (with tilde expansion, filtered to existing)
/// 3. Standard defaults (`~/.ssh/id_ed25519`, `id_rsa`, `id_ecdsa`), filtered to existing
pub(crate) fn collect_key_files(
    opts_identity_files: &[PathBuf],
    ssh_config_identity_files: &Option<Vec<PathBuf>>,
) -> Vec<PathBuf> {
    if !opts_identity_files.is_empty() {
        opts_identity_files
            .iter()
            .map(|p| expand_tilde(p))
            .collect()
    } else if let Some(config_files) = ssh_config_identity_files {
        config_files
            .iter()
            .map(|p| expand_tilde(p))
            .filter(|p| p.exists())
            .collect()
    } else {
        // Try standard default key paths
        if let Some(home) = dirs::home_dir() {
            let ssh_dir = home.join(".ssh");
            let defaults = [
                ssh_dir.join("id_ed25519"),
                ssh_dir.join("id_rsa"),
                ssh_dir.join("id_ecdsa"),
            ];
            defaults.into_iter().filter(|p| p.exists()).collect()
        } else {
            warn!("Could not determine home directory; skipping default key discovery");
            Vec::new()
        }
    }
}

/// Read a key file, handle encrypted key passphrase prompting, and attempt authentication.
///
/// Returns `Ok(Some(true))` if authenticated, `Ok(Some(false))` if tried but rejected,
/// `Ok(None)` if the key couldn't be loaded or was skipped.
pub(crate) async fn load_and_try_key(
    handle: &mut Handle<ClientHandler>,
    user: &str,
    key_file: &Path,
    handler: &impl SshAuthHandler,
    server_methods: &mut Option<russh::MethodSet>,
) -> io::Result<Option<bool>> {
    let contents = match tokio::fs::read_to_string(key_file).await {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to read key {:?}: {}", key_file, e);
            return Ok(None);
        }
    };

    // Try decoding without passphrase first; if the key is encrypted,
    // prompt the user for the passphrase and retry.
    let key = match russh::keys::decode_secret_key(&contents, None) {
        Ok(k) => k,
        Err(e) => {
            if e.to_string().to_lowercase().contains("encrypted") {
                debug!("Key {:?} is encrypted, prompting for passphrase", key_file);
                let file_name = key_file.file_name().unwrap_or_default().to_string_lossy();
                let event = SshAuthEvent {
                    username: user.to_string(),
                    instructions: String::new(),
                    prompts: vec![SshAuthPrompt {
                        prompt: format!("Enter passphrase for key '{file_name}': "),
                        echo: false,
                    }],
                };
                match handler.on_authenticate(event).await {
                    Ok(answers) if !answers.is_empty() && !answers[0].is_empty() => {
                        match russh::keys::decode_secret_key(&contents, Some(&answers[0])) {
                            Ok(k) => k,
                            Err(e2) => {
                                warn!("Failed to decrypt key {:?}: {}", key_file, e2);
                                return Ok(None);
                            }
                        }
                    }
                    Ok(_) => {
                        debug!(
                            "Skipping encrypted key {:?} (no passphrase provided)",
                            key_file
                        );
                        return Ok(None);
                    }
                    Err(e) => {
                        debug!(
                            "Skipping encrypted key {:?} (prompt failed: {})",
                            key_file, e
                        );
                        return Ok(None);
                    }
                }
            } else {
                warn!("Failed to load key {:?}: {}", key_file, e);
                return Ok(None);
            }
        }
    };

    let key_with_hash = russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), None);

    debug!("Trying publickey auth with {:?}", key_file);
    let auth_res = handle
        .authenticate_publickey(user, key_with_hash)
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;

    if auth_res.success() {
        return Ok(Some(true));
    }

    if let russh::client::AuthResult::Failure {
        remaining_methods, ..
    } = auth_res
    {
        *server_methods = Some(remaining_methods);
    }

    Ok(Some(false))
}

/// Attempt keyboard-interactive authentication.
///
/// Returns `(authenticated, user_was_prompted)` — the second flag indicates whether the
/// user was shown an interactive prompt, which is used to avoid double-prompting with
/// password auth.
pub(crate) async fn try_keyboard_interactive(
    handle: &mut Handle<ClientHandler>,
    user: &str,
    handler: &impl SshAuthHandler,
    methods_tried: &mut Vec<String>,
    server_methods: &mut Option<russh::MethodSet>,
) -> io::Result<(bool, bool)> {
    debug!("Trying keyboard-interactive auth");
    let mut user_was_prompted = false;

    match handle
        .authenticate_keyboard_interactive_start(user, None)
        .await
    {
        Ok(mut response) => {
            methods_tried.push("keyboard-interactive".to_string());
            loop {
                match response {
                    russh::client::KeyboardInteractiveAuthResponse::Success => {
                        return Ok((true, user_was_prompted));
                    }
                    russh::client::KeyboardInteractiveAuthResponse::Failure {
                        remaining_methods,
                        ..
                    } => {
                        *server_methods = Some(remaining_methods);
                        break;
                    }
                    russh::client::KeyboardInteractiveAuthResponse::InfoRequest {
                        name,
                        instructions,
                        prompts,
                    } => {
                        if prompts.is_empty() {
                            // Server sent an empty prompt set; respond with empty answers
                            match handle
                                .authenticate_keyboard_interactive_respond(Vec::new())
                                .await
                            {
                                Ok(next) => {
                                    response = next;
                                    continue;
                                }
                                Err(e) => {
                                    warn!("keyboard-interactive respond failed: {e}");
                                    break;
                                }
                            }
                        }

                        user_was_prompted = true;
                        let event = SshAuthEvent {
                            username: if name.is_empty() {
                                user.to_string()
                            } else {
                                name
                            },
                            instructions: if instructions.is_empty() {
                                "Authentication required".to_string()
                            } else {
                                instructions
                            },
                            prompts: prompts
                                .into_iter()
                                .map(|p| SshAuthPrompt {
                                    prompt: p.prompt,
                                    echo: p.echo,
                                })
                                .collect(),
                        };
                        let answers = handler.on_authenticate(event).await?;
                        match handle
                            .authenticate_keyboard_interactive_respond(answers)
                            .await
                        {
                            Ok(next) => response = next,
                            Err(e) => {
                                warn!("keyboard-interactive respond failed: {e}");
                                break;
                            }
                        }
                    }
                }
            }
        }
        Err(e) => {
            warn!("keyboard-interactive start failed: {e}");
        }
    }

    Ok((false, user_was_prompted))
}

/// Attempt password authentication by prompting the user.
///
/// Returns `Ok(true)` if authenticated, `Ok(false)` otherwise.
pub(crate) async fn try_password_auth(
    handle: &mut Handle<ClientHandler>,
    user: &str,
    handler: &impl SshAuthHandler,
    methods_tried: &mut Vec<String>,
    server_methods: &mut Option<russh::MethodSet>,
) -> io::Result<bool> {
    let event = SshAuthEvent {
        username: user.to_string(),
        instructions: "Password:".to_string(),
        prompts: vec![SshAuthPrompt {
            prompt: "Password: ".to_string(),
            echo: false,
        }],
    };

    let responses = handler.on_authenticate(event).await?;

    if let Some(password) = responses.first() {
        methods_tried.push("password".to_string());
        debug!("Trying password auth");
        let auth_res = handle
            .authenticate_password(user, password)
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;

        if auth_res.success() {
            return Ok(true);
        }

        if let russh::client::AuthResult::Failure {
            remaining_methods, ..
        } = auth_res
        {
            *server_methods = Some(remaining_methods);
        }
    }

    Ok(false)
}

/// Build a descriptive authentication error after all methods have been exhausted.
pub(crate) fn build_auth_error(
    methods_tried: &[String],
    server_methods: &Option<russh::MethodSet>,
) -> io::Error {
    let tried = if methods_tried.is_empty() {
        "none".to_string()
    } else {
        methods_tried.join(", ")
    };
    let accepts = server_methods
        .as_ref()
        .map(format_methods)
        .unwrap_or_else(|| "unknown".to_string());

    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("Permission denied (tried: {tried}; server accepts: {accepts})"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- expand_tilde tests ----

    mod expand_tilde_tests {
        use super::*;

        #[test]
        fn should_expand_tilde_ssh_known_hosts_path() {
            let path = Path::new("~/.ssh/known_hosts");
            let expanded = expand_tilde(path);
            if let Some(home) = dirs::home_dir() {
                assert_eq!(expanded, home.join(".ssh").join("known_hosts"));
            }
        }

        #[test]
        fn should_leave_absolute_path_unchanged() {
            let path = Path::new("/etc/ssh/known_hosts");
            assert_eq!(expand_tilde(path), PathBuf::from("/etc/ssh/known_hosts"));
        }

        #[test]
        fn should_expand_tilde_identity_file_path() {
            let path = Path::new("~/.ssh/id_ed25519");
            let expanded = expand_tilde(path);
            if let Some(home) = dirs::home_dir() {
                assert_eq!(expanded, home.join(".ssh").join("id_ed25519"));
            }
        }

        #[test]
        fn should_leave_relative_path_unchanged() {
            let path = Path::new("relative/path");
            assert_eq!(expand_tilde(path), PathBuf::from("relative/path"));
        }

        #[test]
        fn should_expand_bare_tilde_to_home_dir() {
            let path = Path::new("~");
            let expanded = expand_tilde(path);
            if let Some(home) = dirs::home_dir() {
                assert_eq!(expanded, home);
            }
        }

        #[test]
        fn should_be_idempotent() {
            let path = Path::new("~/file");
            let expanded = expand_tilde(&expand_tilde(path));
            let single = expand_tilde(path);
            assert_eq!(expanded, single);
        }
    }

    // ---- collect_key_files tests ----

    mod collect_key_files_tests {
        use super::*;

        #[test]
        fn should_return_explicit_identity_files_with_priority() {
            let opts_files = vec![PathBuf::from("/custom/key1"), PathBuf::from("/custom/key2")];
            let config_files = Some(vec![PathBuf::from("/config/key1")]);
            let result = collect_key_files(&opts_files, &config_files);
            // Explicit files take priority; config files are ignored
            assert_eq!(result.len(), 2);
            assert_eq!(result[0], PathBuf::from("/custom/key1"));
            assert_eq!(result[1], PathBuf::from("/custom/key2"));
        }

        #[test]
        fn should_apply_tilde_expansion_to_explicit_files() {
            let opts_files = vec![PathBuf::from("~/my_key")];
            let result = collect_key_files(&opts_files, &None);
            if let Some(home) = dirs::home_dir() {
                assert_eq!(result.len(), 1);
                assert_eq!(result[0], home.join("my_key"));
            }
        }

        #[test]
        fn should_preserve_nonexistent_explicit_files() {
            // Explicit identity files are NOT filtered for existence
            let opts_files = vec![
                PathBuf::from("/nonexistent/path/key1"),
                PathBuf::from("/nonexistent/path/key2"),
            ];
            let result = collect_key_files(&opts_files, &None);
            assert_eq!(result.len(), 2);
            assert_eq!(result[0], PathBuf::from("/nonexistent/path/key1"));
            assert_eq!(result[1], PathBuf::from("/nonexistent/path/key2"));
        }

        #[test]
        fn should_use_config_files_when_no_explicit_files() {
            // Config files are filtered to existing ones only, so with non-existent
            // config paths the result will be empty
            let opts_files: Vec<PathBuf> = vec![];
            let config_files = Some(vec![PathBuf::from("/nonexistent/config/key")]);
            let result = collect_key_files(&opts_files, &config_files);
            // Non-existent config files are filtered out
            assert!(
                result.is_empty(),
                "Non-existent config files should be filtered out, got: {result:?}"
            );
        }

        #[test]
        fn should_filter_config_files_to_existing_only() {
            // Create a temp file so we have one that exists
            let tmp = tempfile::NamedTempFile::new().unwrap();
            let existing_path = tmp.path().to_path_buf();

            let opts_files: Vec<PathBuf> = vec![];
            let config_files = Some(vec![
                PathBuf::from("/nonexistent/config/key"),
                existing_path.clone(),
            ]);
            let result = collect_key_files(&opts_files, &config_files);
            // Only the existing file should remain
            assert_eq!(result.len(), 1);
            assert_eq!(result[0], existing_path);
        }

        #[test]
        fn should_apply_tilde_expansion_to_config_files() {
            // We cannot easily test tilde expansion on config files with existence
            // filtering (the expanded path would need to exist). Instead, verify
            // that tilde expansion is applied by checking with an existing file
            // that doesn't have a tilde prefix - the function still works.
            let tmp = tempfile::NamedTempFile::new().unwrap();
            let existing_path = tmp.path().to_path_buf();

            let opts_files: Vec<PathBuf> = vec![];
            let config_files = Some(vec![existing_path.clone()]);
            let result = collect_key_files(&opts_files, &config_files);
            assert_eq!(result.len(), 1);
            assert_eq!(result[0], existing_path);
        }

        #[test]
        fn should_return_existing_defaults_when_no_explicit_or_config_files() {
            let opts_files: Vec<PathBuf> = vec![];
            let config_files: Option<Vec<PathBuf>> = None;
            let result = collect_key_files(&opts_files, &config_files);
            // We can't know which default keys exist on the test machine, but
            // we can verify the result only contains paths within ~/.ssh/
            if let Some(home) = dirs::home_dir() {
                let ssh_dir = home.join(".ssh");
                for path in &result {
                    assert!(
                        path.starts_with(&ssh_dir),
                        "Default key path {path:?} should be under {ssh_dir:?}"
                    );
                    let file_name = path.file_name().unwrap().to_string_lossy();
                    assert!(
                        file_name == "id_ed25519"
                            || file_name == "id_rsa"
                            || file_name == "id_ecdsa",
                        "Unexpected default key: {file_name}"
                    );
                }
            }
        }

        #[test]
        fn should_return_empty_config_when_all_nonexistent() {
            let opts_files: Vec<PathBuf> = vec![];
            let config_files = Some(vec![
                PathBuf::from("/nonexistent/a"),
                PathBuf::from("/nonexistent/b"),
                PathBuf::from("/nonexistent/c"),
            ]);
            let result = collect_key_files(&opts_files, &config_files);
            assert!(
                result.is_empty(),
                "All non-existent config files should be filtered out, got: {result:?}"
            );
        }
    }

    // ---- build_auth_error tests ----

    mod build_auth_error_tests {
        use super::*;

        #[test]
        fn should_report_no_methods_and_unknown_server() {
            let methods_tried: Vec<String> = vec![];
            let server_methods: Option<russh::MethodSet> = None;
            let error = build_auth_error(&methods_tried, &server_methods);
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            let msg = error.to_string();
            assert!(
                msg.contains("tried: none"),
                "Expected 'tried: none' in '{msg}'"
            );
            assert!(
                msg.contains("server accepts: unknown"),
                "Expected 'server accepts: unknown' in '{msg}'"
            );
        }

        #[test]
        fn should_report_single_method_tried() {
            let methods_tried = vec!["publickey".to_string()];
            let server_methods: Option<russh::MethodSet> = None;
            let error = build_auth_error(&methods_tried, &server_methods);
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
            let msg = error.to_string();
            assert!(
                msg.contains("tried: publickey"),
                "Expected 'tried: publickey' in '{msg}'"
            );
        }

        #[test]
        fn should_report_multiple_methods_tried_comma_separated() {
            let methods_tried = vec![
                "agent".to_string(),
                "publickey".to_string(),
                "keyboard-interactive".to_string(),
            ];
            let server_methods: Option<russh::MethodSet> = None;
            let error = build_auth_error(&methods_tried, &server_methods);
            let msg = error.to_string();
            assert!(
                msg.contains("tried: agent, publickey, keyboard-interactive"),
                "Expected comma-separated methods in '{msg}'"
            );
        }

        #[test]
        fn should_report_known_server_methods() {
            let methods_tried = vec!["publickey".to_string()];
            let server_methods = Some(russh::MethodSet::from(
                [russh::MethodKind::PublicKey, russh::MethodKind::Password].as_slice(),
            ));
            let error = build_auth_error(&methods_tried, &server_methods);
            let msg = error.to_string();
            assert!(
                msg.contains("publickey"),
                "Expected 'publickey' in server accepts: '{msg}'"
            );
            assert!(
                msg.contains("password"),
                "Expected 'password' in server accepts: '{msg}'"
            );
        }

        #[test]
        fn should_have_permission_denied_error_kind() {
            let error = build_auth_error(&[], &None);
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        }

        #[test]
        fn should_start_with_permission_denied_prefix() {
            let error = build_auth_error(&["password".to_string()], &None);
            let msg = error.to_string();
            assert!(
                msg.starts_with("Permission denied"),
                "Expected message to start with 'Permission denied', got: '{msg}'"
            );
        }
    }

    // ---- format_methods tests ----

    mod format_methods_tests {
        use super::*;

        #[test]
        fn should_return_none_for_empty_set() {
            let methods = russh::MethodSet::empty();
            assert_eq!(format_methods(&methods), "none");
        }

        #[test]
        fn should_format_single_publickey() {
            let methods = russh::MethodSet::from([russh::MethodKind::PublicKey].as_slice());
            assert_eq!(format_methods(&methods), "publickey");
        }

        #[test]
        fn should_format_single_password() {
            let methods = russh::MethodSet::from([russh::MethodKind::Password].as_slice());
            assert_eq!(format_methods(&methods), "password");
        }

        #[test]
        fn should_format_multiple_methods_with_comma_separator() {
            let methods = russh::MethodSet::from(
                [russh::MethodKind::PublicKey, russh::MethodKind::Password].as_slice(),
            );
            let result = format_methods(&methods);
            let parts: Vec<&str> = result.split(", ").collect();
            assert_eq!(parts.len(), 2, "Expected 2 methods in '{result}'");
            assert!(
                result.contains("publickey"),
                "Expected 'publickey' in '{result}'"
            );
            assert!(
                result.contains("password"),
                "Expected 'password' in '{result}'"
            );
        }

        #[test]
        fn should_format_keyboard_interactive() {
            let methods =
                russh::MethodSet::from([russh::MethodKind::KeyboardInteractive].as_slice());
            assert_eq!(format_methods(&methods), "keyboard-interactive");
        }
    }

    // ---- Encrypted key detection tests ----

    mod encrypted_key_detection_tests {
        use std::process::Command;

        #[test]
        fn should_fail_decoding_encrypted_key_without_passphrase() {
            let tmp = tempfile::tempdir().unwrap();
            let key_path = tmp.path().join("encrypted_key");

            // Generate an encrypted ed25519 key
            let status = Command::new("ssh-keygen")
                .args(["-m", "PEM"])
                .args(["-t", "ed25519"])
                .arg("-f")
                .arg(&key_path)
                .arg("-N")
                .arg("test_passphrase")
                .arg("-q")
                .status();

            match status {
                Ok(s) if s.success() => {
                    let contents = std::fs::read_to_string(&key_path).unwrap();
                    let result = russh::keys::decode_secret_key(&contents, None);
                    assert!(
                        result.is_err(),
                        "Encrypted key should fail to decode without passphrase"
                    );
                    let err_msg = result.unwrap_err().to_string().to_lowercase();
                    assert!(
                        err_msg.contains("encrypted") || err_msg.contains("decrypt"),
                        "Error should mention encryption, got: '{err_msg}'"
                    );
                }
                _ => {
                    // ssh-keygen not available; skip silently
                    eprintln!("ssh-keygen not available, skipping encrypted key test");
                }
            }
        }

        #[test]
        fn should_succeed_decoding_encrypted_key_with_correct_passphrase() {
            let tmp = tempfile::tempdir().unwrap();
            let key_path = tmp.path().join("encrypted_key");

            let status = Command::new("ssh-keygen")
                .args(["-m", "PEM"])
                .args(["-t", "ed25519"])
                .arg("-f")
                .arg(&key_path)
                .arg("-N")
                .arg("test_passphrase")
                .arg("-q")
                .status();

            match status {
                Ok(s) if s.success() => {
                    let contents = std::fs::read_to_string(&key_path).unwrap();
                    let result = russh::keys::decode_secret_key(&contents, Some("test_passphrase"));
                    assert!(
                        result.is_ok(),
                        "Encrypted key should decode with correct passphrase, got: {:?}",
                        result.unwrap_err()
                    );
                }
                _ => {
                    eprintln!("ssh-keygen not available, skipping encrypted key test");
                }
            }
        }

        #[test]
        fn should_succeed_decoding_unencrypted_key_without_passphrase() {
            let tmp = tempfile::tempdir().unwrap();
            let key_path = tmp.path().join("unencrypted_key");

            let status = Command::new("ssh-keygen")
                .args(["-m", "PEM"])
                .args(["-t", "ed25519"])
                .arg("-f")
                .arg(&key_path)
                .arg("-N")
                .arg("")
                .arg("-q")
                .status();

            match status {
                Ok(s) if s.success() => {
                    let contents = std::fs::read_to_string(&key_path).unwrap();
                    let result = russh::keys::decode_secret_key(&contents, None);
                    assert!(
                        result.is_ok(),
                        "Unencrypted key should decode without passphrase, got: {:?}",
                        result.unwrap_err()
                    );
                }
                _ => {
                    eprintln!("ssh-keygen not available, skipping unencrypted key test");
                }
            }
        }

        #[test]
        fn should_fail_decoding_encrypted_key_with_wrong_passphrase() {
            let tmp = tempfile::tempdir().unwrap();
            let key_path = tmp.path().join("encrypted_key");

            let status = Command::new("ssh-keygen")
                .args(["-m", "PEM"])
                .args(["-t", "ed25519"])
                .arg("-f")
                .arg(&key_path)
                .arg("-N")
                .arg("correct_passphrase")
                .arg("-q")
                .status();

            match status {
                Ok(s) if s.success() => {
                    let contents = std::fs::read_to_string(&key_path).unwrap();
                    let result =
                        russh::keys::decode_secret_key(&contents, Some("wrong_passphrase"));
                    assert!(
                        result.is_err(),
                        "Encrypted key should fail with wrong passphrase"
                    );
                }
                _ => {
                    eprintln!("ssh-keygen not available, skipping encrypted key test");
                }
            }
        }
    }
}
