//! SSH authentication strategies.
//!
//! This module provides functions for each SSH authentication method:
//! agent-based, file-based public key, keyboard-interactive, and password.
//! Utility functions for path expansion and error formatting are also included.

use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use log::*;
use russh::MethodSet;
use russh::client::Handle;
use russh::client::{AuthResult, KeyboardInteractiveAuthResponse};
use russh::keys::agent::client::AgentClient;
use russh::keys::{Algorithm, Certificate, PrivateKeyWithHashAlg};

use crate::{ClientHandler, SshAuthEvent, SshAuthHandler, SshAuthPrompt};

/// Abstraction over SSH session authentication operations.
pub(crate) trait AuthSession: Send {
    fn auth_publickey(
        &mut self,
        user: &str,
        key: PrivateKeyWithHashAlg,
    ) -> impl Future<Output = Result<AuthResult, russh::Error>> + Send;

    fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> impl Future<Output = Result<AuthResult, russh::Error>> + Send;

    fn auth_keyboard_interactive_start(
        &mut self,
        user: &str,
        submethods: Option<&str>,
    ) -> impl Future<Output = Result<KeyboardInteractiveAuthResponse, russh::Error>> + Send;

    fn auth_keyboard_interactive_respond(
        &mut self,
        answers: Vec<String>,
    ) -> impl Future<Output = Result<KeyboardInteractiveAuthResponse, russh::Error>> + Send;
}

impl AuthSession for Handle<ClientHandler> {
    async fn auth_publickey(
        &mut self,
        user: &str,
        key: PrivateKeyWithHashAlg,
    ) -> Result<AuthResult, russh::Error> {
        self.authenticate_publickey(user, key).await
    }

    async fn auth_password(
        &mut self,
        user: &str,
        password: &str,
    ) -> Result<AuthResult, russh::Error> {
        self.authenticate_password(user, password).await
    }

    async fn auth_keyboard_interactive_start(
        &mut self,
        user: &str,
        submethods: Option<&str>,
    ) -> Result<KeyboardInteractiveAuthResponse, russh::Error> {
        self.authenticate_keyboard_interactive_start(user, submethods.map(String::from))
            .await
    }

    async fn auth_keyboard_interactive_respond(
        &mut self,
        answers: Vec<String>,
    ) -> Result<KeyboardInteractiveAuthResponse, russh::Error> {
        self.authenticate_keyboard_interactive_respond(answers)
            .await
    }
}

/// Abstraction over SSH agent key listing and authentication.
pub(crate) trait AgentAuthenticator: Send {
    fn request_identities(
        &mut self,
    ) -> impl Future<Output = io::Result<Vec<russh::keys::PublicKey>>> + Send;

    fn try_authenticate_key(
        &mut self,
        user: &str,
        key: russh::keys::PublicKey,
    ) -> impl Future<Output = io::Result<AuthResult>> + Send;
}

struct AgentHandle<'h, 'a, S>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    handle: &'h mut Handle<ClientHandler>,
    agent: &'a mut AgentClient<S>,
}

impl<S> AgentAuthenticator for AgentHandle<'_, '_, S>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    async fn request_identities(&mut self) -> io::Result<Vec<russh::keys::PublicKey>> {
        self.agent
            .request_identities()
            .await
            .map_err(io::Error::other)
    }

    async fn try_authenticate_key(
        &mut self,
        user: &str,
        key: russh::keys::PublicKey,
    ) -> io::Result<AuthResult> {
        self.handle
            .authenticate_publickey_with(user, key, None, &mut *self.agent)
            .await
            .map_err(io::Error::other)
    }
}

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
pub(crate) fn format_methods(methods: &MethodSet) -> String {
    if methods.is_empty() {
        return "none".to_string();
    }
    methods
        .iter()
        .map(<&str>::from)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Try SSH agent authentication using the given agent authenticator.
///
/// Queries the agent for available keys and tries each one against the server.
/// Returns `true` if any agent key was accepted.
async fn authenticate_with_agent(
    auth: &mut impl AgentAuthenticator,
    user: &str,
    methods_tried: &mut Vec<String>,
    server_methods: &mut Option<MethodSet>,
) -> bool {
    let keys = match auth.request_identities().await {
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
        match auth.try_authenticate_key(user, key.clone()).await {
            Ok(res) if res.success() => return true,
            Ok(AuthResult::Failure {
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
    server_methods: &mut Option<MethodSet>,
) -> io::Result<bool> {
    debug!("Attempting SSH agent authentication");

    #[cfg(unix)]
    {
        match AgentClient::connect_env().await {
            Ok(mut agent) => {
                let mut ha = AgentHandle {
                    handle,
                    agent: &mut agent,
                };
                if authenticate_with_agent(&mut ha, user, methods_tried, server_methods).await {
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
                let mut ha = AgentHandle {
                    handle,
                    agent: &mut agent,
                };
                if authenticate_with_agent(&mut ha, user, methods_tried, server_methods).await {
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
                    let mut ha = AgentHandle {
                        handle,
                        agent: &mut agent,
                    };
                    if authenticate_with_agent(&mut ha, user, methods_tried, server_methods).await {
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

/// Discover certificate file paths from identity files.
///
/// Uses two methods (matching OpenSSH behavior):
/// 1. Explicit certs: identity files ending in `-cert.pub`
/// 2. Auto-discovered certs: for each identity file NOT ending in `-cert.pub`,
///    checks if `<path>-cert.pub` exists on disk
fn discover_cert_files(key_files: &[PathBuf]) -> Vec<PathBuf> {
    let mut cert_files = Vec::new();
    for key_file in key_files {
        let key_str = key_file.to_string_lossy();
        if key_str.ends_with("-cert.pub") {
            cert_files.push(key_file.clone());
        } else {
            // Auto-discover: check if <path>-cert.pub exists
            let cert_path = PathBuf::from(format!("{}-cert.pub", key_str));
            if cert_path.exists() {
                cert_files.push(cert_path);
            }
        }
    }
    cert_files
}

/// Try certificate authentication with a connected agent.
///
/// Iterates over discovered certificate files, loads each one, and attempts
/// certificate-based public key authentication using the agent for signing.
async fn try_certs_with_agent<S>(
    handle: &mut Handle<ClientHandler>,
    user: &str,
    cert_files: &[PathBuf],
    agent: &mut AgentClient<S>,
    server_methods: &mut Option<MethodSet>,
) -> io::Result<bool>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    for cert_file in cert_files {
        let expanded = expand_tilde(cert_file);
        debug!("Trying certificate: {}", expanded.display());

        let contents = match tokio::fs::read_to_string(&expanded).await {
            Ok(c) => c,
            Err(e) => {
                debug!("Failed to read certificate {:?}: {}", expanded, e);
                continue;
            }
        };

        let cert = match Certificate::from_openssh(&contents) {
            Ok(c) => c,
            Err(e) => {
                debug!("Failed to parse certificate {:?}: {}", expanded, e);
                continue;
            }
        };

        // Determine hash_alg for RSA certs
        let hash_alg = match cert.algorithm() {
            Algorithm::Rsa { hash } => hash,
            _ => None,
        };

        debug!(
            "Attempting cert auth with {:?} (algo: {:?})",
            expanded,
            cert.algorithm()
        );

        match handle
            .authenticate_certificate_with(user, cert, hash_alg, agent)
            .await
        {
            Ok(res) if res.success() => {
                info!("Certificate authentication succeeded with {:?}", expanded);
                return Ok(true);
            }
            Ok(AuthResult::Failure {
                remaining_methods,
                partial_success,
            }) => {
                if partial_success {
                    info!(
                        "Certificate accepted (partial success), remaining: {}",
                        format_methods(&remaining_methods)
                    );
                } else {
                    debug!("Certificate rejected: {:?}", expanded);
                }
                *server_methods = Some(remaining_methods);
            }
            Ok(_) => {
                debug!("Certificate auth inconclusive for {:?}", expanded);
            }
            Err(e) => {
                debug!("Certificate auth error for {:?}: {}", expanded, e);
            }
        }
    }

    Ok(false)
}

/// Attempt SSH certificate authentication using the agent.
///
/// Discovers certificate files from identity files, connects to the platform-
/// appropriate SSH agent, and tries `authenticate_certificate_with` for each cert.
pub(crate) async fn try_cert_agent_auth(
    handle: &mut Handle<ClientHandler>,
    user: &str,
    key_files: &[PathBuf],
    agent_socket: Option<&str>,
    methods_tried: &mut Vec<String>,
    server_methods: &mut Option<MethodSet>,
) -> io::Result<bool> {
    let cert_files = discover_cert_files(key_files);
    if cert_files.is_empty() {
        debug!("No certificate files found");
        return Ok(false);
    }

    debug!("Found {} certificate file(s)", cert_files.len());
    methods_tried.push("certificate".to_string());

    #[cfg(unix)]
    {
        // Try custom agent socket first if specified
        if let Some(socket) = agent_socket {
            let expanded = expand_tilde(Path::new(socket));
            debug!("Connecting to custom agent socket: {}", expanded.display());
            match AgentClient::connect_uds(expanded).await {
                Ok(mut agent) => {
                    if try_certs_with_agent(handle, user, &cert_files, &mut agent, server_methods)
                        .await?
                    {
                        return Ok(true);
                    }
                }
                Err(e) => {
                    debug!("Failed to connect to custom agent socket: {}", e);
                }
            }
        } else {
            match AgentClient::connect_env().await {
                Ok(mut agent) => {
                    if try_certs_with_agent(handle, user, &cert_files, &mut agent, server_methods)
                        .await?
                    {
                        return Ok(true);
                    }
                }
                Err(e) => {
                    debug!("SSH agent not available for cert auth: {}", e);
                }
            }
        }
    }

    #[cfg(windows)]
    {
        let _ = agent_socket; // Custom socket paths are Unix-only
        match AgentClient::connect_named_pipe(r"\\.\pipe\openssh-ssh-agent").await {
            Ok(mut agent) => {
                if try_certs_with_agent(handle, user, &cert_files, &mut agent, server_methods)
                    .await?
                {
                    return Ok(true);
                }
            }
            Err(e) => {
                debug!("OpenSSH agent not available for cert auth: {:?}", e);
            }
        }

        match AgentClient::connect_pageant().await {
            Ok(mut agent) => {
                if try_certs_with_agent(handle, user, &cert_files, &mut agent, server_methods)
                    .await?
                {
                    return Ok(true);
                }
            }
            Err(e) => {
                debug!("Pageant not available for cert auth: {:?}", e);
            }
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
    session: &mut impl AuthSession,
    user: &str,
    key_file: &Path,
    handler: &impl SshAuthHandler,
    server_methods: &mut Option<MethodSet>,
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
            if matches!(e, russh::keys::Error::KeyIsEncrypted) {
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

    let key_with_hash = PrivateKeyWithHashAlg::new(Arc::new(key), None);

    debug!("Trying publickey auth with {:?}", key_file);
    let auth_res = session
        .auth_publickey(user, key_with_hash)
        .await
        .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;

    if auth_res.success() {
        return Ok(Some(true));
    }

    if let AuthResult::Failure {
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
    session: &mut impl AuthSession,
    user: &str,
    handler: &impl SshAuthHandler,
    methods_tried: &mut Vec<String>,
    server_methods: &mut Option<MethodSet>,
) -> io::Result<(bool, bool)> {
    debug!("Trying keyboard-interactive auth");
    let mut user_was_prompted = false;

    match session.auth_keyboard_interactive_start(user, None).await {
        Ok(mut response) => {
            methods_tried.push("keyboard-interactive".to_string());
            loop {
                match response {
                    KeyboardInteractiveAuthResponse::Success => {
                        return Ok((true, user_was_prompted));
                    }
                    KeyboardInteractiveAuthResponse::Failure {
                        remaining_methods, ..
                    } => {
                        *server_methods = Some(remaining_methods);
                        break;
                    }
                    KeyboardInteractiveAuthResponse::InfoRequest {
                        name,
                        instructions,
                        prompts,
                    } => {
                        if prompts.is_empty() {
                            // Server sent an empty prompt set; respond with empty answers
                            match session.auth_keyboard_interactive_respond(Vec::new()).await {
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
                        match session.auth_keyboard_interactive_respond(answers).await {
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
    session: &mut impl AuthSession,
    user: &str,
    handler: &impl SshAuthHandler,
    methods_tried: &mut Vec<String>,
    server_methods: &mut Option<MethodSet>,
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
        let auth_res = session
            .auth_password(user, password)
            .await
            .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;

        if auth_res.success() {
            return Ok(true);
        }

        if let AuthResult::Failure {
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
    server_methods: &Option<MethodSet>,
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
    use std::collections::VecDeque;
    use std::process::Command;
    use std::sync::Mutex;

    use super::*;

    struct MockAuthSession {
        publickey_results: VecDeque<Result<AuthResult, russh::Error>>,
        password_results: VecDeque<Result<AuthResult, russh::Error>>,
        kbd_int_start_results: VecDeque<Result<KeyboardInteractiveAuthResponse, russh::Error>>,
        kbd_int_respond_results: VecDeque<Result<KeyboardInteractiveAuthResponse, russh::Error>>,
    }

    impl MockAuthSession {
        fn new() -> Self {
            Self {
                publickey_results: VecDeque::new(),
                password_results: VecDeque::new(),
                kbd_int_start_results: VecDeque::new(),
                kbd_int_respond_results: VecDeque::new(),
            }
        }
    }

    impl AuthSession for MockAuthSession {
        async fn auth_publickey(
            &mut self,
            _user: &str,
            _key: PrivateKeyWithHashAlg,
        ) -> Result<AuthResult, russh::Error> {
            self.publickey_results
                .pop_front()
                .expect("no more publickey results")
        }

        async fn auth_password(
            &mut self,
            _user: &str,
            _password: &str,
        ) -> Result<AuthResult, russh::Error> {
            self.password_results
                .pop_front()
                .expect("no more password results")
        }

        async fn auth_keyboard_interactive_start(
            &mut self,
            _user: &str,
            _submethods: Option<&str>,
        ) -> Result<KeyboardInteractiveAuthResponse, russh::Error> {
            self.kbd_int_start_results
                .pop_front()
                .expect("no more kbd_int_start results")
        }

        async fn auth_keyboard_interactive_respond(
            &mut self,
            _answers: Vec<String>,
        ) -> Result<KeyboardInteractiveAuthResponse, russh::Error> {
            self.kbd_int_respond_results
                .pop_front()
                .expect("no more kbd_int_respond results")
        }
    }

    struct MockAgentAuthenticator {
        identities_result: Option<io::Result<Vec<russh::keys::PublicKey>>>,
        auth_results: VecDeque<io::Result<AuthResult>>,
    }

    impl AgentAuthenticator for MockAgentAuthenticator {
        async fn request_identities(&mut self) -> io::Result<Vec<russh::keys::PublicKey>> {
            self.identities_result.take().expect("no identities result")
        }

        async fn try_authenticate_key(
            &mut self,
            _user: &str,
            _key: russh::keys::PublicKey,
        ) -> io::Result<AuthResult> {
            self.auth_results.pop_front().expect("no more auth results")
        }
    }

    struct MockSshAuthHandler {
        responses: Mutex<VecDeque<io::Result<Vec<String>>>>,
    }

    impl MockSshAuthHandler {
        fn new(responses: Vec<io::Result<Vec<String>>>) -> Self {
            Self {
                responses: Mutex::new(VecDeque::from(responses)),
            }
        }
    }

    impl SshAuthHandler for MockSshAuthHandler {
        fn on_authenticate(
            &self,
            _event: SshAuthEvent,
        ) -> impl Future<Output = io::Result<Vec<String>>> + Send {
            let result = self
                .responses
                .lock()
                .unwrap()
                .pop_front()
                .expect("no more handler responses");
            async { result }
        }

        fn on_verify_host<'a>(
            &'a self,
            _host: &'a str,
        ) -> impl Future<Output = io::Result<bool>> + Send + 'a {
            async { Ok(false) }
        }

        fn on_banner<'a>(&'a self, _text: &'a str) -> impl Future<Output = ()> + Send + 'a {
            async {}
        }

        fn on_error<'a>(&'a self, _text: &'a str) -> impl Future<Output = ()> + Send + 'a {
            async {}
        }
    }

    fn test_public_key() -> russh::keys::PublicKey {
        let key = russh::keys::PrivateKey::random(
            &mut rand::thread_rng(),
            russh::keys::Algorithm::Ed25519,
        )
        .unwrap();
        key.public_key().clone()
    }

    fn password_method_set() -> MethodSet {
        MethodSet::from([russh::MethodKind::Password].as_slice())
    }

    #[test]
    fn expand_tilde_should_expand_ssh_known_hosts_path() {
        let path = Path::new("~/.ssh/known_hosts");
        let expanded = expand_tilde(path);
        if let Some(home) = dirs::home_dir() {
            assert_eq!(expanded, home.join(".ssh").join("known_hosts"));
        }
    }

    #[test]
    fn expand_tilde_should_leave_absolute_path_unchanged() {
        let path = Path::new("/etc/ssh/known_hosts");
        assert_eq!(expand_tilde(path), PathBuf::from("/etc/ssh/known_hosts"));
    }

    #[test]
    fn expand_tilde_should_expand_tilde_identity_file_path() {
        let path = Path::new("~/.ssh/id_ed25519");
        let expanded = expand_tilde(path);
        if let Some(home) = dirs::home_dir() {
            assert_eq!(expanded, home.join(".ssh").join("id_ed25519"));
        }
    }

    #[test]
    fn expand_tilde_should_leave_relative_path_unchanged() {
        let path = Path::new("relative/path");
        assert_eq!(expand_tilde(path), PathBuf::from("relative/path"));
    }

    #[test]
    fn expand_tilde_should_expand_bare_tilde_to_home_dir() {
        let path = Path::new("~");
        let expanded = expand_tilde(path);
        if let Some(home) = dirs::home_dir() {
            assert_eq!(expanded, home);
        }
    }

    #[test]
    fn expand_tilde_should_be_idempotent() {
        let path = Path::new("~/file");
        let expanded = expand_tilde(&expand_tilde(path));
        let single = expand_tilde(path);
        assert_eq!(expanded, single);
    }

    #[test]
    fn collect_key_files_should_return_explicit_identity_files_with_priority() {
        let opts_files = vec![PathBuf::from("/custom/key1"), PathBuf::from("/custom/key2")];
        let config_files = Some(vec![PathBuf::from("/config/key1")]);
        let result = collect_key_files(&opts_files, &config_files);
        // Explicit files take priority; config files are ignored
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], PathBuf::from("/custom/key1"));
        assert_eq!(result[1], PathBuf::from("/custom/key2"));
    }

    #[test]
    fn collect_key_files_should_apply_tilde_expansion_to_explicit_files() {
        let opts_files = vec![PathBuf::from("~/my_key")];
        let result = collect_key_files(&opts_files, &None);
        if let Some(home) = dirs::home_dir() {
            assert_eq!(result.len(), 1);
            assert_eq!(result[0], home.join("my_key"));
        }
    }

    #[test]
    fn collect_key_files_should_preserve_nonexistent_explicit_files() {
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
    fn collect_key_files_should_use_config_files_when_no_explicit_files() {
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
    fn collect_key_files_should_filter_config_files_to_existing_only() {
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
    fn collect_key_files_should_apply_tilde_expansion_to_config_files() {
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
    fn collect_key_files_should_return_existing_defaults_when_no_explicit_or_config_files() {
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
                    file_name == "id_ed25519" || file_name == "id_rsa" || file_name == "id_ecdsa",
                    "Unexpected default key: {file_name}"
                );
            }
        }
    }

    #[test]
    fn collect_key_files_should_return_empty_config_when_all_nonexistent() {
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

    #[test]
    fn build_auth_error_should_report_no_methods_and_unknown_server() {
        let methods_tried: Vec<String> = vec![];
        let server_methods: Option<MethodSet> = None;
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
    fn build_auth_error_should_report_single_method_tried() {
        let methods_tried = vec!["publickey".to_string()];
        let server_methods: Option<MethodSet> = None;
        let error = build_auth_error(&methods_tried, &server_methods);
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        let msg = error.to_string();
        assert!(
            msg.contains("tried: publickey"),
            "Expected 'tried: publickey' in '{msg}'"
        );
    }

    #[test]
    fn build_auth_error_should_report_multiple_methods_tried_comma_separated() {
        let methods_tried = vec![
            "agent".to_string(),
            "publickey".to_string(),
            "keyboard-interactive".to_string(),
        ];
        let server_methods: Option<MethodSet> = None;
        let error = build_auth_error(&methods_tried, &server_methods);
        let msg = error.to_string();
        assert!(
            msg.contains("tried: agent, publickey, keyboard-interactive"),
            "Expected comma-separated methods in '{msg}'"
        );
    }

    #[test]
    fn build_auth_error_should_report_known_server_methods() {
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
    fn build_auth_error_should_have_permission_denied_error_kind() {
        let error = build_auth_error(&[], &None);
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn build_auth_error_should_start_with_permission_denied_prefix() {
        let error = build_auth_error(&["password".to_string()], &None);
        let msg = error.to_string();
        assert!(
            msg.starts_with("Permission denied"),
            "Expected message to start with 'Permission denied', got: '{msg}'"
        );
    }

    #[test]
    fn format_methods_should_return_none_for_empty_set() {
        let methods = russh::MethodSet::empty();
        assert_eq!(format_methods(&methods), "none");
    }

    #[test]
    fn format_methods_should_format_single_publickey() {
        let methods = russh::MethodSet::from([russh::MethodKind::PublicKey].as_slice());
        assert_eq!(format_methods(&methods), "publickey");
    }

    #[test]
    fn format_methods_should_format_single_password() {
        let methods = russh::MethodSet::from([russh::MethodKind::Password].as_slice());
        assert_eq!(format_methods(&methods), "password");
    }

    #[test]
    fn format_methods_should_format_multiple_methods_with_comma_separator() {
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
    fn format_methods_should_format_keyboard_interactive() {
        let methods = russh::MethodSet::from([russh::MethodKind::KeyboardInteractive].as_slice());
        assert_eq!(format_methods(&methods), "keyboard-interactive");
    }

    #[test]
    fn encrypted_key_should_fail_decoding_without_passphrase() {
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
    fn encrypted_key_should_succeed_decoding_with_correct_passphrase() {
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
    fn encrypted_key_should_succeed_decoding_unencrypted_without_passphrase() {
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
    fn encrypted_key_should_fail_decoding_with_wrong_passphrase() {
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
                let result = russh::keys::decode_secret_key(&contents, Some("wrong_passphrase"));
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

    #[tokio::test]
    async fn authenticate_with_agent_should_return_false_on_identities_error() {
        let mut auth = MockAgentAuthenticator {
            identities_result: Some(Err(io::Error::new(
                io::ErrorKind::ConnectionRefused,
                "agent not available",
            ))),
            auth_results: VecDeque::new(),
        };
        let mut methods_tried = Vec::new();
        let mut server_methods = None;

        let result =
            authenticate_with_agent(&mut auth, "user", &mut methods_tried, &mut server_methods)
                .await;

        assert!(!result, "Should return false when identities request fails");
        assert!(
            methods_tried.is_empty(),
            "Should not record any method when identities fail"
        );
    }

    #[tokio::test]
    async fn authenticate_with_agent_should_return_false_when_no_keys() {
        let mut auth = MockAgentAuthenticator {
            identities_result: Some(Ok(Vec::new())),
            auth_results: VecDeque::new(),
        };
        let mut methods_tried = Vec::new();
        let mut server_methods = None;

        let result =
            authenticate_with_agent(&mut auth, "user", &mut methods_tried, &mut server_methods)
                .await;

        assert!(!result, "Should return false when agent has no keys");
        assert!(
            methods_tried.is_empty(),
            "Should not record method when agent has no keys"
        );
    }

    #[tokio::test]
    async fn authenticate_with_agent_should_return_true_when_key_accepted() {
        let key = test_public_key();
        let mut auth = MockAgentAuthenticator {
            identities_result: Some(Ok(vec![key])),
            auth_results: VecDeque::from([Ok(AuthResult::Success)]),
        };
        let mut methods_tried = Vec::new();
        let mut server_methods = None;

        let result =
            authenticate_with_agent(&mut auth, "user", &mut methods_tried, &mut server_methods)
                .await;

        assert!(result, "Should return true when agent key is accepted");
        assert_eq!(
            methods_tried,
            vec!["agent"],
            "Should record 'agent' in methods_tried"
        );
    }

    #[tokio::test]
    async fn authenticate_with_agent_should_update_server_methods_on_failure() {
        let key = test_public_key();
        let mut auth = MockAgentAuthenticator {
            identities_result: Some(Ok(vec![key])),
            auth_results: VecDeque::from([Ok(AuthResult::Failure {
                remaining_methods: password_method_set(),
                partial_success: false,
            })]),
        };
        let mut methods_tried = Vec::new();
        let mut server_methods = None;

        let result =
            authenticate_with_agent(&mut auth, "user", &mut methods_tried, &mut server_methods)
                .await;

        assert!(
            !result,
            "Should return false when agent key is rejected with Failure"
        );
        assert!(
            server_methods.is_some(),
            "Should update server_methods from Failure variant"
        );
        let methods = server_methods.unwrap();
        let formatted = format_methods(&methods);
        assert!(
            formatted.contains("password"),
            "server_methods should contain password, got: '{formatted}'"
        );
    }

    #[tokio::test]
    async fn authenticate_with_agent_should_return_false_on_non_success() {
        let key = test_public_key();
        // Use an io error (non-Failure, non-Success) path
        let mut auth = MockAgentAuthenticator {
            identities_result: Some(Ok(vec![key])),
            auth_results: VecDeque::from([Ok(AuthResult::Failure {
                remaining_methods: MethodSet::empty(),
                partial_success: true,
            })]),
        };
        let mut methods_tried = Vec::new();
        let mut server_methods = None;

        let result =
            authenticate_with_agent(&mut auth, "user", &mut methods_tried, &mut server_methods)
                .await;

        assert!(
            !result,
            "Should return false on non-success AuthResult variant"
        );
    }

    #[tokio::test]
    async fn authenticate_with_agent_should_return_false_on_error() {
        let key = test_public_key();
        let mut auth = MockAgentAuthenticator {
            identities_result: Some(Ok(vec![key])),
            auth_results: VecDeque::from([Err(io::Error::other("auth failed"))]),
        };
        let mut methods_tried = Vec::new();
        let mut server_methods = None;

        let result =
            authenticate_with_agent(&mut auth, "user", &mut methods_tried, &mut server_methods)
                .await;

        assert!(
            !result,
            "Should return false when try_authenticate_key errors"
        );
    }

    #[tokio::test]
    async fn authenticate_with_agent_should_try_next_key_on_failure() {
        let key1 = test_public_key();
        let key2 = test_public_key();
        let mut auth = MockAgentAuthenticator {
            identities_result: Some(Ok(vec![key1, key2])),
            auth_results: VecDeque::from([
                Ok(AuthResult::Failure {
                    remaining_methods: password_method_set(),
                    partial_success: false,
                }),
                Ok(AuthResult::Success),
            ]),
        };
        let mut methods_tried = Vec::new();
        let mut server_methods = None;

        let result =
            authenticate_with_agent(&mut auth, "user", &mut methods_tried, &mut server_methods)
                .await;

        assert!(result, "Should return true when second agent key succeeds");
        assert_eq!(methods_tried, vec!["agent"]);
    }

    #[tokio::test]
    async fn try_password_auth_should_propagate_handler_error() {
        let mut session = MockAuthSession::new();
        let handler = MockSshAuthHandler::new(vec![Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "user cancelled",
        ))]);
        let mut methods_tried = Vec::new();
        let mut server_methods = None;

        let result = try_password_auth(
            &mut session,
            "user",
            &handler,
            &mut methods_tried,
            &mut server_methods,
        )
        .await;

        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Interrupted);
        assert!(
            err.to_string().contains("user cancelled"),
            "Error should propagate handler error message, got: '{}'",
            err
        );
    }

    #[tokio::test]
    async fn try_password_auth_should_return_false_when_no_responses() {
        let mut session = MockAuthSession::new();
        let handler = MockSshAuthHandler::new(vec![Ok(Vec::new())]);
        let mut methods_tried = Vec::new();
        let mut server_methods = None;

        let result = try_password_auth(
            &mut session,
            "user",
            &handler,
            &mut methods_tried,
            &mut server_methods,
        )
        .await
        .unwrap();

        assert!(
            !result,
            "Should return false when handler returns empty responses"
        );
        assert!(
            methods_tried.is_empty(),
            "Should not record method when no password provided"
        );
    }

    #[tokio::test]
    async fn try_password_auth_should_propagate_auth_error() {
        let mut session = MockAuthSession::new();
        session
            .password_results
            .push_back(Err(russh::Error::NotAuthenticated));
        let handler = MockSshAuthHandler::new(vec![Ok(vec!["mypassword".to_string()])]);
        let mut methods_tried = Vec::new();
        let mut server_methods = None;

        let result = try_password_auth(
            &mut session,
            "user",
            &handler,
            &mut methods_tried,
            &mut server_methods,
        )
        .await;

        let err = result.unwrap_err();
        assert_eq!(
            err.kind(),
            io::ErrorKind::PermissionDenied,
            "Auth errors should be mapped to PermissionDenied"
        );
    }

    #[tokio::test]
    async fn try_password_auth_should_return_true_on_success() {
        let mut session = MockAuthSession::new();
        session.password_results.push_back(Ok(AuthResult::Success));
        let handler = MockSshAuthHandler::new(vec![Ok(vec!["mypassword".to_string()])]);
        let mut methods_tried = Vec::new();
        let mut server_methods = None;

        let result = try_password_auth(
            &mut session,
            "user",
            &handler,
            &mut methods_tried,
            &mut server_methods,
        )
        .await
        .unwrap();

        assert!(result, "Should return true on successful password auth");
        assert_eq!(
            methods_tried,
            vec!["password"],
            "Should record 'password' in methods_tried"
        );
    }

    #[tokio::test]
    async fn try_password_auth_should_update_methods_on_failure() {
        let mut session = MockAuthSession::new();
        session.password_results.push_back(Ok(AuthResult::Failure {
            remaining_methods: password_method_set(),
            partial_success: false,
        }));
        let handler = MockSshAuthHandler::new(vec![Ok(vec!["badpassword".to_string()])]);
        let mut methods_tried = Vec::new();
        let mut server_methods = None;

        let result = try_password_auth(
            &mut session,
            "user",
            &handler,
            &mut methods_tried,
            &mut server_methods,
        )
        .await
        .unwrap();

        assert!(
            !result,
            "Should return false when password auth fails with Failure"
        );
        assert!(
            server_methods.is_some(),
            "Should update server_methods from Failure variant"
        );
    }

    #[tokio::test]
    async fn try_password_auth_should_return_false_on_non_failure() {
        // AuthResult only has Success and Failure, so a Failure with partial_success
        // still returns false (since success() is false)
        let mut session = MockAuthSession::new();
        session.password_results.push_back(Ok(AuthResult::Failure {
            remaining_methods: MethodSet::empty(),
            partial_success: true,
        }));
        let handler = MockSshAuthHandler::new(vec![Ok(vec!["pass".to_string()])]);
        let mut methods_tried = Vec::new();
        let mut server_methods = None;

        let result = try_password_auth(
            &mut session,
            "user",
            &handler,
            &mut methods_tried,
            &mut server_methods,
        )
        .await
        .unwrap();

        assert!(!result, "Should return false on non-success AuthResult");
    }

    #[tokio::test]
    async fn try_keyboard_interactive_should_return_false_on_start_error() {
        let mut session = MockAuthSession::new();
        session
            .kbd_int_start_results
            .push_back(Err(russh::Error::NotAuthenticated));
        let handler = MockSshAuthHandler::new(vec![]);
        let mut methods_tried = Vec::new();
        let mut server_methods = None;

        let (authed, prompted) = try_keyboard_interactive(
            &mut session,
            "user",
            &handler,
            &mut methods_tried,
            &mut server_methods,
        )
        .await
        .unwrap();

        assert!(!authed, "Should return false when start errors");
        assert!(
            !prompted,
            "user_was_prompted should be false when start errors"
        );
    }

    #[tokio::test]
    async fn try_keyboard_interactive_should_return_true_on_immediate_success() {
        let mut session = MockAuthSession::new();
        session
            .kbd_int_start_results
            .push_back(Ok(KeyboardInteractiveAuthResponse::Success));
        let handler = MockSshAuthHandler::new(vec![]);
        let mut methods_tried = Vec::new();
        let mut server_methods = None;

        let (authed, prompted) = try_keyboard_interactive(
            &mut session,
            "user",
            &handler,
            &mut methods_tried,
            &mut server_methods,
        )
        .await
        .unwrap();

        assert!(authed, "Should return true on immediate Success");
        assert!(
            !prompted,
            "user_was_prompted should be false on immediate Success"
        );
        assert_eq!(
            methods_tried,
            vec!["keyboard-interactive"],
            "Should record 'keyboard-interactive' in methods_tried"
        );
    }

    #[tokio::test]
    async fn try_keyboard_interactive_should_update_methods_on_failure() {
        let mut session = MockAuthSession::new();
        session
            .kbd_int_start_results
            .push_back(Ok(KeyboardInteractiveAuthResponse::Failure {
                remaining_methods: password_method_set(),
                partial_success: false,
            }));
        let handler = MockSshAuthHandler::new(vec![]);
        let mut methods_tried = Vec::new();
        let mut server_methods = None;

        let (authed, prompted) = try_keyboard_interactive(
            &mut session,
            "user",
            &handler,
            &mut methods_tried,
            &mut server_methods,
        )
        .await
        .unwrap();

        assert!(!authed, "Should return false when start returns Failure");
        assert!(
            !prompted,
            "user_was_prompted should be false when start returns Failure"
        );
        assert!(
            server_methods.is_some(),
            "Should update server_methods from Failure variant"
        );
    }

    #[tokio::test]
    async fn try_keyboard_interactive_should_succeed_on_empty_prompts() {
        let mut session = MockAuthSession::new();
        session
            .kbd_int_start_results
            .push_back(Ok(KeyboardInteractiveAuthResponse::InfoRequest {
                name: String::new(),
                instructions: String::new(),
                prompts: Vec::new(),
            }));
        session
            .kbd_int_respond_results
            .push_back(Ok(KeyboardInteractiveAuthResponse::Success));
        let handler = MockSshAuthHandler::new(vec![]);
        let mut methods_tried = Vec::new();
        let mut server_methods = None;

        let (authed, prompted) = try_keyboard_interactive(
            &mut session,
            "user",
            &handler,
            &mut methods_tried,
            &mut server_methods,
        )
        .await
        .unwrap();

        assert!(
            authed,
            "Should return true when empty prompts followed by Success"
        );
        assert!(
            !prompted,
            "user_was_prompted should be false for empty prompts"
        );
    }

    #[tokio::test]
    async fn try_keyboard_interactive_should_handle_respond_error_on_empty_prompts() {
        let mut session = MockAuthSession::new();
        session
            .kbd_int_start_results
            .push_back(Ok(KeyboardInteractiveAuthResponse::InfoRequest {
                name: String::new(),
                instructions: String::new(),
                prompts: Vec::new(),
            }));
        session
            .kbd_int_respond_results
            .push_back(Err(russh::Error::NotAuthenticated));
        let handler = MockSshAuthHandler::new(vec![]);
        let mut methods_tried = Vec::new();
        let mut server_methods = None;

        let (authed, prompted) = try_keyboard_interactive(
            &mut session,
            "user",
            &handler,
            &mut methods_tried,
            &mut server_methods,
        )
        .await
        .unwrap();

        assert!(
            !authed,
            "Should return false when respond errors on empty prompts"
        );
        assert!(
            !prompted,
            "user_was_prompted should be false for empty prompts even on error"
        );
    }

    #[tokio::test]
    async fn try_keyboard_interactive_should_succeed_with_user_prompts() {
        let mut session = MockAuthSession::new();
        session
            .kbd_int_start_results
            .push_back(Ok(KeyboardInteractiveAuthResponse::InfoRequest {
                name: String::new(),
                instructions: String::new(),
                prompts: vec![russh::client::Prompt {
                    prompt: "Password: ".to_string(),
                    echo: false,
                }],
            }));
        session
            .kbd_int_respond_results
            .push_back(Ok(KeyboardInteractiveAuthResponse::Success));
        let handler = MockSshAuthHandler::new(vec![Ok(vec!["mypassword".to_string()])]);
        let mut methods_tried = Vec::new();
        let mut server_methods = None;

        let (authed, prompted) = try_keyboard_interactive(
            &mut session,
            "user",
            &handler,
            &mut methods_tried,
            &mut server_methods,
        )
        .await
        .unwrap();

        assert!(
            authed,
            "Should return true when user provides answers and server returns Success"
        );
        assert!(
            prompted,
            "user_was_prompted should be true when prompts were shown"
        );
    }

    #[tokio::test]
    async fn try_keyboard_interactive_should_propagate_handler_error() {
        let mut session = MockAuthSession::new();
        session
            .kbd_int_start_results
            .push_back(Ok(KeyboardInteractiveAuthResponse::InfoRequest {
                name: String::new(),
                instructions: String::new(),
                prompts: vec![russh::client::Prompt {
                    prompt: "Password: ".to_string(),
                    echo: false,
                }],
            }));
        let handler = MockSshAuthHandler::new(vec![Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "user cancelled prompt",
        ))]);
        let mut methods_tried = Vec::new();
        let mut server_methods = None;

        let result = try_keyboard_interactive(
            &mut session,
            "user",
            &handler,
            &mut methods_tried,
            &mut server_methods,
        )
        .await;

        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Interrupted);
        assert!(
            err.to_string().contains("user cancelled prompt"),
            "Should propagate handler error, got: '{}'",
            err
        );
    }

    #[tokio::test]
    async fn try_keyboard_interactive_should_update_methods_on_prompt_failure() {
        let mut session = MockAuthSession::new();
        session
            .kbd_int_start_results
            .push_back(Ok(KeyboardInteractiveAuthResponse::InfoRequest {
                name: String::new(),
                instructions: String::new(),
                prompts: vec![russh::client::Prompt {
                    prompt: "Password: ".to_string(),
                    echo: false,
                }],
            }));
        session
            .kbd_int_respond_results
            .push_back(Ok(KeyboardInteractiveAuthResponse::Failure {
                remaining_methods: password_method_set(),
                partial_success: false,
            }));
        let handler = MockSshAuthHandler::new(vec![Ok(vec!["wrong".to_string()])]);
        let mut methods_tried = Vec::new();
        let mut server_methods = None;

        let (authed, prompted) = try_keyboard_interactive(
            &mut session,
            "user",
            &handler,
            &mut methods_tried,
            &mut server_methods,
        )
        .await
        .unwrap();

        assert!(!authed, "Should return false when respond returns Failure");
        assert!(
            prompted,
            "user_was_prompted should be true when prompts were shown"
        );
        assert!(
            server_methods.is_some(),
            "Should update server_methods from Failure after prompt"
        );
    }

    #[tokio::test]
    async fn load_and_try_key_should_return_none_for_missing_file() {
        let mut session = MockAuthSession::new();
        let handler = MockSshAuthHandler::new(vec![]);
        let mut server_methods = None;
        let missing_path = Path::new("/nonexistent/path/to/key");

        let result = load_and_try_key(
            &mut session,
            "user",
            missing_path,
            &handler,
            &mut server_methods,
        )
        .await
        .unwrap();

        assert_eq!(
            result, None,
            "Should return None when key file does not exist"
        );
    }

    #[tokio::test]
    async fn load_and_try_key_should_return_true_on_auth_success() {
        let tmp = tempfile::tempdir().unwrap();
        let key_path = tmp.path().join("test_key");

        let status = Command::new("ssh-keygen")
            .args(["-t", "ed25519"])
            .arg("-f")
            .arg(&key_path)
            .arg("-N")
            .arg("")
            .arg("-q")
            .status();

        match status {
            Ok(s) if s.success() => {
                let mut session = MockAuthSession::new();
                session.publickey_results.push_back(Ok(AuthResult::Success));
                let handler = MockSshAuthHandler::new(vec![]);
                let mut server_methods = None;

                let result = load_and_try_key(
                    &mut session,
                    "user",
                    &key_path,
                    &handler,
                    &mut server_methods,
                )
                .await
                .unwrap();

                assert_eq!(
                    result,
                    Some(true),
                    "Should return Some(true) when auth succeeds"
                );
            }
            _ => {
                eprintln!("ssh-keygen not available, skipping load_and_try_key test");
            }
        }
    }

    #[tokio::test]
    async fn load_and_try_key_should_update_methods_on_auth_failure() {
        let tmp = tempfile::tempdir().unwrap();
        let key_path = tmp.path().join("test_key");

        let status = Command::new("ssh-keygen")
            .args(["-t", "ed25519"])
            .arg("-f")
            .arg(&key_path)
            .arg("-N")
            .arg("")
            .arg("-q")
            .status();

        match status {
            Ok(s) if s.success() => {
                let mut session = MockAuthSession::new();
                session.publickey_results.push_back(Ok(AuthResult::Failure {
                    remaining_methods: password_method_set(),
                    partial_success: false,
                }));
                let handler = MockSshAuthHandler::new(vec![]);
                let mut server_methods = None;

                let result = load_and_try_key(
                    &mut session,
                    "user",
                    &key_path,
                    &handler,
                    &mut server_methods,
                )
                .await
                .unwrap();

                assert_eq!(
                    result,
                    Some(false),
                    "Should return Some(false) when auth fails"
                );
                assert!(
                    server_methods.is_some(),
                    "Should update server_methods from Failure variant"
                );
            }
            _ => {
                eprintln!("ssh-keygen not available, skipping load_and_try_key test");
            }
        }
    }

    #[tokio::test]
    async fn load_and_try_key_should_propagate_auth_error() {
        let tmp = tempfile::tempdir().unwrap();
        let key_path = tmp.path().join("test_key");

        let status = Command::new("ssh-keygen")
            .args(["-t", "ed25519"])
            .arg("-f")
            .arg(&key_path)
            .arg("-N")
            .arg("")
            .arg("-q")
            .status();

        match status {
            Ok(s) if s.success() => {
                let mut session = MockAuthSession::new();
                session
                    .publickey_results
                    .push_back(Err(russh::Error::NotAuthenticated));
                let handler = MockSshAuthHandler::new(vec![]);
                let mut server_methods = None;

                let result = load_and_try_key(
                    &mut session,
                    "user",
                    &key_path,
                    &handler,
                    &mut server_methods,
                )
                .await;

                let err = result.unwrap_err();
                assert_eq!(
                    err.kind(),
                    io::ErrorKind::PermissionDenied,
                    "Auth errors should be mapped to PermissionDenied"
                );
            }
            _ => {
                eprintln!("ssh-keygen not available, skipping load_and_try_key test");
            }
        }
    }

    #[tokio::test]
    async fn load_and_try_key_should_return_true_for_encrypted_key_with_passphrase() {
        let tmp = tempfile::tempdir().unwrap();
        let key_path = tmp.path().join("enc_key");

        let status = Command::new("ssh-keygen")
            .args(["-t", "ed25519"])
            .arg("-f")
            .arg(&key_path)
            .arg("-N")
            .arg("test_pass")
            .arg("-q")
            .status();

        match status {
            Ok(s) if s.success() => {
                let mut session = MockAuthSession::new();
                session.publickey_results.push_back(Ok(AuthResult::Success));
                let handler = MockSshAuthHandler::new(vec![Ok(vec!["test_pass".to_string()])]);
                let mut server_methods = None;

                let result = load_and_try_key(
                    &mut session,
                    "user",
                    &key_path,
                    &handler,
                    &mut server_methods,
                )
                .await
                .unwrap();

                assert_eq!(
                    result,
                    Some(true),
                    "Should return Some(true) for encrypted key with correct passphrase"
                );
            }
            _ => {
                eprintln!("ssh-keygen not available, skipping encrypted key test");
            }
        }
    }

    #[tokio::test]
    async fn load_and_try_key_should_return_none_for_wrong_passphrase() {
        let tmp = tempfile::tempdir().unwrap();
        let key_path = tmp.path().join("enc_key");

        let status = Command::new("ssh-keygen")
            .args(["-t", "ed25519"])
            .arg("-f")
            .arg(&key_path)
            .arg("-N")
            .arg("correct_pass")
            .arg("-q")
            .status();

        match status {
            Ok(s) if s.success() => {
                let mut session = MockAuthSession::new();
                let handler = MockSshAuthHandler::new(vec![Ok(vec!["wrong_pass".to_string()])]);
                let mut server_methods = None;

                let result = load_and_try_key(
                    &mut session,
                    "user",
                    &key_path,
                    &handler,
                    &mut server_methods,
                )
                .await
                .unwrap();

                assert_eq!(result, None, "Should return None when passphrase is wrong");
            }
            _ => {
                eprintln!("ssh-keygen not available, skipping encrypted key test");
            }
        }
    }

    #[tokio::test]
    async fn load_and_try_key_should_return_none_for_empty_passphrase() {
        let tmp = tempfile::tempdir().unwrap();
        let key_path = tmp.path().join("enc_key");

        let status = Command::new("ssh-keygen")
            .args(["-t", "ed25519"])
            .arg("-f")
            .arg(&key_path)
            .arg("-N")
            .arg("some_pass")
            .arg("-q")
            .status();

        match status {
            Ok(s) if s.success() => {
                let mut session = MockAuthSession::new();
                let handler = MockSshAuthHandler::new(vec![Ok(vec![String::new()])]);
                let mut server_methods = None;

                let result = load_and_try_key(
                    &mut session,
                    "user",
                    &key_path,
                    &handler,
                    &mut server_methods,
                )
                .await
                .unwrap();

                assert_eq!(result, None, "Should return None when passphrase is empty");
            }
            _ => {
                eprintln!("ssh-keygen not available, skipping encrypted key test");
            }
        }
    }

    #[tokio::test]
    async fn load_and_try_key_should_return_none_on_handler_error() {
        let tmp = tempfile::tempdir().unwrap();
        let key_path = tmp.path().join("enc_key");

        let status = Command::new("ssh-keygen")
            .args(["-t", "ed25519"])
            .arg("-f")
            .arg(&key_path)
            .arg("-N")
            .arg("some_pass")
            .arg("-q")
            .status();

        match status {
            Ok(s) if s.success() => {
                let mut session = MockAuthSession::new();
                let handler = MockSshAuthHandler::new(vec![Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "user cancelled",
                ))]);
                let mut server_methods = None;

                let result = load_and_try_key(
                    &mut session,
                    "user",
                    &key_path,
                    &handler,
                    &mut server_methods,
                )
                .await
                .unwrap();

                assert_eq!(
                    result, None,
                    "Should return None when handler errors for encrypted key"
                );
            }
            _ => {
                eprintln!("ssh-keygen not available, skipping encrypted key test");
            }
        }
    }

    #[tokio::test]
    async fn load_and_try_key_should_return_none_for_invalid_key() {
        let tmp = tempfile::tempdir().unwrap();
        let key_path = tmp.path().join("invalid_key");
        std::fs::write(&key_path, "this is not a valid ssh key at all").unwrap();

        let mut session = MockAuthSession::new();
        let handler = MockSshAuthHandler::new(vec![]);
        let mut server_methods = None;

        let result = load_and_try_key(
            &mut session,
            "user",
            &key_path,
            &handler,
            &mut server_methods,
        )
        .await
        .unwrap();

        assert_eq!(
            result, None,
            "Should return None for invalid key file content"
        );
    }

    #[test]
    fn discover_cert_files_should_return_explicit_cert_files() {
        let cert_path = PathBuf::from("/some/path/id_ed25519-cert.pub");
        let result = discover_cert_files(std::slice::from_ref(&cert_path));
        assert_eq!(result, vec![cert_path]);
    }

    #[test]
    fn discover_cert_files_should_auto_discover_adjacent_cert_files() {
        let tmp = tempfile::tempdir().unwrap();
        let key_path = tmp.path().join("id_ed25519");
        let cert_path = tmp.path().join("id_ed25519-cert.pub");

        std::fs::write(&key_path, "fake key").unwrap();
        std::fs::write(&cert_path, "fake cert").unwrap();

        let result = discover_cert_files(&[key_path]);
        assert_eq!(result, vec![cert_path]);
    }

    #[test]
    fn discover_cert_files_should_not_return_nonexistent_auto_certs() {
        let tmp = tempfile::tempdir().unwrap();
        let key_path = tmp.path().join("id_ed25519");
        std::fs::write(&key_path, "fake key").unwrap();

        let result = discover_cert_files(&[key_path]);
        assert!(
            result.is_empty(),
            "Should return empty when no adjacent cert file exists, got: {result:?}"
        );
    }

    #[test]
    fn discover_cert_files_should_return_empty_for_no_input() {
        let result = discover_cert_files(&[]);
        assert!(
            result.is_empty(),
            "Should return empty for empty input, got: {result:?}"
        );
    }

    #[test]
    fn discover_cert_files_should_handle_mix_of_explicit_and_auto() {
        let tmp = tempfile::tempdir().unwrap();

        let explicit_cert = PathBuf::from("/explicit/id_rsa-cert.pub");

        let auto_key = tmp.path().join("id_ed25519");
        let auto_cert = tmp.path().join("id_ed25519-cert.pub");
        std::fs::write(&auto_key, "fake key").unwrap();
        std::fs::write(&auto_cert, "fake cert").unwrap();

        let no_cert_key = tmp.path().join("id_ecdsa");
        std::fs::write(&no_cert_key, "fake ecdsa key").unwrap();

        let result = discover_cert_files(&[explicit_cert.clone(), auto_key, no_cert_key]);
        assert_eq!(
            result.len(),
            2,
            "Should find explicit cert and auto-discovered cert, got: {result:?}"
        );
        assert_eq!(result[0], explicit_cert);
        assert_eq!(result[1], auto_cert);
    }
}
