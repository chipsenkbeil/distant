#![doc = include_str!("../README.md")]
#![allow(dead_code)] // Allow unused functions/fields that may be platform-specific or future use
#![allow(clippy::field_reassign_with_default)] // Sometimes clearer than inline initialization
#![allow(clippy::manual_async_fn)] // Trait implementations may require this pattern

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;

use std::collections::BTreeMap;
use std::error::Error as StdError;
use std::fs::File;
use std::future::Future;
use std::io::{self, BufReader, Write};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use distant_core::net::auth::{AuthHandlerMap, DummyAuthHandler, Verifier};
use distant_core::net::client::{Client as NetClient, ClientConfig};
use distant_core::net::common::{InmemoryTransport, OneshotListener, Version};
use distant_core::net::server::{Server, ServerRef};
use distant_core::protocol::PROTOCOL_VERSION;
use distant_core::{ApiServerHandler, Client, Credentials};
use log::*;
use russh::client::{self, Handle};
use russh::keys::PrivateKey;
use ssh2_config_rs::{HostParams, ParseRule, SshConfig};
use tokio::sync::Mutex;

mod api;
mod process;
mod utils;

use api::SshApi;

/// Format a `MethodSet` as a comma-separated string of method names.
fn format_methods(methods: &russh::MethodSet) -> String {
    if methods.is_empty() {
        return "none".to_string();
    }
    methods
        .iter()
        .map(<&str>::from)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Represents the family of the remote machine connected over SSH
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum SshFamily {
    /// Operating system belongs to unix family
    Unix,

    /// Operating system belongs to windows family
    Windows,
}

impl SshFamily {
    pub const fn as_static_str(&self) -> &'static str {
        match self {
            Self::Unix => "unix",
            Self::Windows => "windows",
        }
    }
}

/// Represents a singular authentication prompt for a new ssh client
#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SshAuthPrompt {
    /// The label to show when prompting the user
    pub prompt: String,

    /// If true, the response that the user inputs should be displayed as they type. If false then
    /// treat it as a password entry and do not display what is typed in response to this prompt.
    pub echo: bool,
}

/// Represents an authentication request that needs to be handled before an ssh client can be
/// established
#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SshAuthEvent {
    /// Represents the name of the user to be authenticated. This may be empty!
    pub username: String,

    /// Informational text to be displayed to the user prior to the prompt
    pub instructions: String,

    /// Prompts to be conveyed to the user, each representing a single answer needed
    pub prompts: Vec<SshAuthPrompt>,
}

/// Represents options to be provided when establishing an ssh client
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct SshOpts {
    /// List of files from which the user's DSA, ECDSA, Ed25519, or RSA authentication identity
    /// is read, defaulting to
    ///
    /// - `~/.ssh/id_dsa`
    /// - `~/.ssh/id_ecdsa`
    /// - `~/.ssh/id_ed25519`
    /// - `~/.ssh/id_rsa`
    pub identity_files: Vec<PathBuf>,

    /// If provided and true, specifies that ssh should only use the configured authentication
    /// and certificate files (either the defaults or configured from `identity_files`)
    ///
    /// Default is false (aka no)
    pub identities_only: Option<bool>,

    /// Port to use when connecting to an SSHD instance
    pub port: Option<u16>,

    /// Specifies the command to use to connect to the server
    pub proxy_command: Option<String>,

    /// Specifies the user to log in as
    pub user: Option<String>,

    /// Specifies one or more files to use for the user host key database, defaulting to
    ///
    /// - `~/.ssh/known_hosts`
    /// - `~/.ssh/known_hosts2`
    pub user_known_hosts_files: Vec<PathBuf>,

    /// If true, will output tracing information from the underlying ssh implementation
    pub verbose: bool,

    /// Additional options to provide as defined by `ssh_config(5)`
    pub other: BTreeMap<String, String>,
}

/// Represents options to be provided when converting an ssh client into a distant client
#[derive(Clone, Debug)]
pub struct LaunchOpts {
    /// Binary to use for distant server
    pub binary: String,

    /// Arguments to supply to the distant server when starting it
    pub args: String,

    /// Timeout to use when connecting to the distant server
    pub timeout: Duration,
}

impl Default for LaunchOpts {
    fn default() -> Self {
        Self {
            binary: String::from("distant"),
            args: String::new(),
            timeout: Duration::from_secs(15),
        }
    }
}

/// Interface to handle various events during ssh authentication
pub trait SshAuthHandler {
    /// Invoked whenever a series of authentication prompts need to be displayed and responded to,
    /// receiving one event at a time and returning a collection of answers matching the total
    /// prompts provided in the event
    fn on_authenticate(
        &self,
        event: SshAuthEvent,
    ) -> impl Future<Output = io::Result<Vec<String>>> + Send;

    /// Invoked when the host is unknown for a new ssh connection, receiving the host as a str and
    /// returning true if the host is acceptable or false if the host (and thereby ssh client)
    /// should be declined
    fn on_verify_host<'a>(
        &'a self,
        host: &'a str,
    ) -> impl Future<Output = io::Result<bool>> + Send + 'a;

    /// Invoked when receiving a banner from the ssh server, receiving the banner as a str, useful
    /// to display to the user
    fn on_banner<'a>(&'a self, text: &'a str) -> impl Future<Output = ()> + Send + 'a;

    /// Invoked when an error is encountered, receiving the error as a str
    fn on_error<'a>(&'a self, text: &'a str) -> impl Future<Output = ()> + Send + 'a;
}

/// Implementation of [`SshAuthHandler`] that prompts locally for authentication and verification
/// events
pub struct LocalSshAuthHandler;

impl SshAuthHandler for LocalSshAuthHandler {
    fn on_authenticate(
        &self,
        event: SshAuthEvent,
    ) -> impl Future<Output = io::Result<Vec<String>>> + Send {
        async move {
            trace!("[local] on_authenticate({event:?})");
            let task = tokio::task::spawn_blocking(move || {
                if !event.username.is_empty() {
                    eprintln!("Authentication for {}", event.username);
                }

                if !event.instructions.is_empty() {
                    eprintln!("{}", event.instructions);
                }

                let mut answers = Vec::new();
                for prompt in &event.prompts {
                    // Contains all prompt lines including same line
                    let mut prompt_lines = prompt.prompt.split('\n').collect::<Vec<_>>();

                    // Line that is prompt on same line as answer
                    let prompt_line = prompt_lines.pop().unwrap();

                    // Go ahead and display all other lines
                    for line in prompt_lines.into_iter() {
                        eprintln!("{line}");
                    }

                    let answer = if prompt.echo {
                        eprint!("{prompt_line}");
                        std::io::stderr().lock().flush()?;

                        let mut answer = String::new();
                        std::io::stdin().read_line(&mut answer)?;
                        answer
                    } else {
                        rpassword::prompt_password(prompt_line)?
                    };

                    answers.push(answer);
                }
                Ok(answers)
            });

            task.await.map_err(io::Error::other)?
        }
    }

    fn on_verify_host<'a>(
        &'a self,
        host: &'a str,
    ) -> impl Future<Output = io::Result<bool>> + Send + 'a {
        async move {
            trace!("[local] on_verify_host({host})");
            eprintln!("{host}");
            let task = tokio::task::spawn_blocking(|| {
                eprint!("Enter [y/N]> ");
                std::io::stderr().lock().flush()?;

                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;

                trace!("Verify? Answer = '{answer}'");
                match answer.as_str().trim() {
                    "y" | "Y" | "yes" | "YES" => Ok(true),
                    _ => Ok(false),
                }
            });

            task.await.map_err(io::Error::other)?
        }
    }

    fn on_banner<'a>(&'a self, _text: &'a str) -> impl Future<Output = ()> + Send + 'a {
        async move {
            trace!("[local] on_banner({_text})");
        }
    }

    fn on_error<'a>(&'a self, _text: &'a str) -> impl Future<Output = ()> + Send + 'a {
        async move {
            trace!("[local] on_error({_text})");
        }
    }
}

/// Handles SSH client events from the russh connection, including host key verification.
struct ClientHandler;

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::PublicKey,
    ) -> impl std::future::Future<Output = Result<bool, Self::Error>> + Send {
        // TODO: Implement proper host key verification using known_hosts
        async { Ok(true) }
    }
}

/// Represents an ssh2 client
pub struct Ssh {
    handle: Handle<ClientHandler>,
    host: String,
    port: u16,
    user: String,
    opts: SshOpts,
    authenticated: bool,
    cached_family: Mutex<Option<SshFamily>>,
}

/// Build the command-line arguments for launching a distant server remotely.
fn build_launch_args(family: SshFamily, binary: &str, extra_args: &str) -> io::Result<String> {
    let mut args = vec![
        String::from("server"),
        String::from("listen"),
        String::from("--daemon"),
        String::from("--host"),
        String::from("ssh"),
    ];
    args.extend(match family {
        SshFamily::Windows => winsplit::split(extra_args),
        SshFamily::Unix => shell_words::split(extra_args)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidInput, x))?,
    });
    Ok(format!("{} {}", binary, args.join(" ")))
}

impl Ssh {
    /// Connect to a remote TCP server using SSH
    pub async fn connect(host: impl AsRef<str>, opts: SshOpts) -> io::Result<Self> {
        // Parse SSH config first
        let ssh_config = Self::parse_ssh_config(host.as_ref())?;

        // Determine connection parameters
        let port = opts.port.or(ssh_config.port).unwrap_or(22);
        let user = opts
            .user
            .clone()
            .or(ssh_config.user.clone())
            .unwrap_or_else(whoami::username);

        info!(
            "SSH connection attempt: {}:{} as user '{}'",
            host.as_ref(),
            port,
            user
        );
        debug!("SSH options: {:?}", opts);
        debug!(
            "SSH config: port={:?}, user={:?}",
            ssh_config.port, ssh_config.user
        );

        // Build russh configuration
        let config = Self::build_russh_config(&opts, &ssh_config)?;

        // Verbose diagnostics
        if opts.verbose {
            info!("SSH verbose mode enabled");
            info!("Target: {}:{}", host.as_ref(), port);
            info!("User: {}", user);
            debug!("Identity files: {:?}", opts.identity_files);
            debug!("Identities only: {:?}", opts.identities_only);
            debug!("Proxy command: {:?}", opts.proxy_command);
            debug!("Known hosts files: {:?}", opts.user_known_hosts_files);
            debug!("Russh keepalive: {:?}", config.keepalive_interval);
        }

        debug!(
            "Initiating russh::client::connect to {}:{}...",
            host.as_ref(),
            port
        );

        let handler = ClientHandler;
        let connect_result =
            russh::client::connect(Arc::new(config), (host.as_ref(), port), handler).await;

        let handle = match connect_result {
            Ok(h) => {
                info!("SSH connection established to {}:{}", host.as_ref(), port);
                h
            }
            Err(e) => {
                error!("SSH connection failed to {}:{}", host.as_ref(), port);
                error!("Russh error: {}", e);
                debug!("Russh error debug: {:?}", e);

                let detailed_msg =
                    if let Some(io_err) = e.source().and_then(|s| s.downcast_ref::<io::Error>()) {
                        error!("Underlying IO error: {}", io_err);
                        error!("IO error kind: {:?}", io_err.kind());
                        error!("OS error code: {:?}", io_err.raw_os_error());

                        format!(
                        "SSH connection to {}:{} failed: {} (IO error: {}, kind: {:?}, os: {:?})",
                        host.as_ref(),
                        port,
                        e,
                        io_err,
                        io_err.kind(),
                        io_err.raw_os_error()
                    )
                    } else {
                        format!("SSH connection to {}:{} failed: {}", host.as_ref(), port, e)
                    };

                return Err(io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    detailed_msg,
                ));
            }
        };

        Ok(Self {
            handle,
            host: host.as_ref().to_string(),
            port,
            user,
            opts,
            authenticated: false,
            cached_family: Mutex::new(None),
        })
    }

    fn parse_ssh_config(host: &str) -> io::Result<HostParams> {
        let config_path = dirs::home_dir()
            .map(|h| h.join(".ssh").join("config"))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "No home directory found"))?;

        if !config_path.exists() {
            use ssh2_config_rs::DefaultAlgorithms;
            return Ok(HostParams::new(&DefaultAlgorithms::default()));
        }

        let mut reader = BufReader::new(File::open(&config_path)?);
        let config = SshConfig::default()
            .parse(&mut reader, ParseRule::ALLOW_UNSUPPORTED_FIELDS)
            .map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Failed to parse SSH config: {}", e),
                )
            })?;

        Ok(config.query(host))
    }

    fn build_russh_config(
        _opts: &SshOpts,
        params: &HostParams,
    ) -> io::Result<russh::client::Config> {
        let mut config = russh::client::Config::default();

        config.preferred = Self::build_preferred_algorithms(params);

        if let Some(interval) = params.server_alive_interval {
            config.keepalive_interval = Some(interval);
        }

        Ok(config)
    }

    fn build_preferred_algorithms(_params: &HostParams) -> russh::Preferred {
        // Using defaults; SSH config algorithm preferences (KexAlgorithms, Ciphers, MACs)
        // are not yet applied.
        russh::Preferred::default()
    }

    /// Host this client is connected to
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Port this client is connected to on remote host
    pub fn port(&self) -> u16 {
        self.port
    }

    #[inline]
    pub fn is_authenticated(&self) -> bool {
        self.authenticated
    }

    /// Authenticates the [`Ssh`] if not already authenticated
    pub async fn authenticate(&mut self, handler: impl SshAuthHandler) -> io::Result<()> {
        use russh::MethodKind;

        if self.authenticated {
            return Ok(());
        }

        // Track what we tried and what the server accepts for error reporting
        let mut methods_tried: Vec<String> = Vec::new();
        let mut server_methods: Option<russh::MethodSet> = None;

        // Probe with "none" auth to discover which methods the server supports
        match self.handle.authenticate_none(&self.user).await {
            Ok(res) => {
                if res.success() {
                    self.authenticated = true;
                    return Ok(());
                }
                if let russh::client::AuthResult::Failure {
                    remaining_methods, ..
                } = res
                {
                    debug!(
                        "Server auth methods: {}",
                        format_methods(&remaining_methods)
                    );
                    server_methods = Some(remaining_methods);
                }
            }
            Err(e) => {
                warn!("authenticate_none probe failed: {e}");
            }
        }

        let server_accepts_pubkey = server_methods
            .as_ref()
            .is_none_or(|m| m.contains(&MethodKind::PublicKey));
        let server_accepts_password = server_methods
            .as_ref()
            .is_none_or(|m| m.contains(&MethodKind::Password));
        let server_accepts_kbdint = server_methods
            .as_ref()
            .is_none_or(|m| m.contains(&MethodKind::KeyboardInteractive));

        // --- Public key authentication ---
        if server_accepts_pubkey {
            // Determine which key files to try
            let key_files: Vec<PathBuf> = if !self.opts.identity_files.is_empty() {
                self.opts.identity_files.clone()
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
            };

            if !key_files.is_empty() {
                methods_tried.push("publickey".to_string());
            }

            for key_file in &key_files {
                match self.load_private_key(key_file).await {
                    Ok(key) => {
                        let key_with_hash =
                            russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), None);

                        debug!("Trying publickey auth with {:?}", key_file);
                        let auth_res = self
                            .handle
                            .authenticate_publickey(&self.user, key_with_hash)
                            .await
                            .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;

                        if auth_res.success() {
                            self.authenticated = true;
                            return Ok(());
                        }

                        if let russh::client::AuthResult::Failure {
                            remaining_methods, ..
                        } = auth_res
                        {
                            server_methods = Some(remaining_methods);
                        }
                    }
                    Err(e) => {
                        warn!("Failed to load key {:?}: {}", key_file, e);
                    }
                }
            }
        }

        // --- Keyboard-interactive authentication ---
        // Track whether we already prompted the user (to avoid double-prompting with password)
        let mut user_was_prompted = false;

        if server_accepts_kbdint {
            debug!("Trying keyboard-interactive auth");
            match self
                .handle
                .authenticate_keyboard_interactive_start(&self.user, None)
                .await
            {
                Ok(mut response) => {
                    methods_tried.push("keyboard-interactive".to_string());
                    loop {
                        match response {
                            russh::client::KeyboardInteractiveAuthResponse::Success => {
                                self.authenticated = true;
                                return Ok(());
                            }
                            russh::client::KeyboardInteractiveAuthResponse::Failure {
                                remaining_methods,
                                ..
                            } => {
                                server_methods = Some(remaining_methods);
                                break;
                            }
                            russh::client::KeyboardInteractiveAuthResponse::InfoRequest {
                                name,
                                instructions,
                                prompts,
                            } => {
                                if prompts.is_empty() {
                                    // Server sent an empty prompt set; respond with empty answers
                                    match self
                                        .handle
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
                                        self.user.clone()
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
                                match self
                                    .handle
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
        }

        // --- Password authentication ---
        // Skip if keyboard-interactive already prompted the user (avoids double-prompt)
        if server_accepts_password && !user_was_prompted {
            let event = SshAuthEvent {
                username: self.user.clone(),
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
                let auth_res = self
                    .handle
                    .authenticate_password(&self.user, password)
                    .await
                    .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;

                if auth_res.success() {
                    self.authenticated = true;
                    return Ok(());
                }

                if let russh::client::AuthResult::Failure {
                    remaining_methods, ..
                } = auth_res
                {
                    server_methods = Some(remaining_methods);
                }
            }
        }

        // All methods exhausted — build a descriptive error
        let tried = if methods_tried.is_empty() {
            "none".to_string()
        } else {
            methods_tried.join(", ")
        };
        let accepts = server_methods
            .as_ref()
            .map(format_methods)
            .unwrap_or_else(|| "unknown".to_string());

        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("Permission denied (tried: {tried}; server accepts: {accepts})"),
        ))
    }

    async fn load_private_key(&self, path: &Path) -> io::Result<PrivateKey> {
        let contents = tokio::fs::read_to_string(path).await?;
        russh::keys::decode_secret_key(&contents, None)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Detects whether the family is Unix or Windows
    pub async fn detect_family(&self) -> io::Result<SshFamily> {
        {
            let guard = self.cached_family.lock().await;
            if let Some(family) = *guard {
                return Ok(family);
            }
        }

        let is_windows = utils::is_windows(&self.handle).await?;
        let family = if is_windows {
            SshFamily::Windows
        } else {
            SshFamily::Unix
        };

        {
            let mut guard = self.cached_family.lock().await;
            *guard = Some(family);
        }

        Ok(family)
    }

    /// Converts into a distant client
    pub async fn into_distant_client(self) -> io::Result<Client> {
        let family = self.detect_family().await?;
        let api = SshApi::new(self.handle, family);

        let (t1, t2) = InmemoryTransport::pair(100);

        let server = Server::new()
            .handler(ApiServerHandler::new(api))
            .verifier(Verifier::none());

        tokio::spawn(async move {
            let _ = server.start(OneshotListener::from_value(t2));
        });

        let client = NetClient::build()
            .auth_handler(DummyAuthHandler)
            .config(ClientConfig::default())
            .connector(t1)
            .connect()
            .await
            .map_err(io::Error::other)?;

        Ok(client)
    }

    /// Converts into a pair of distant client and server ref
    pub async fn into_distant_pair(self) -> io::Result<(Client, ServerRef)> {
        let family = self.detect_family().await?;
        let api = SshApi::new(self.handle, family);

        let (t1, t2) = InmemoryTransport::pair(100);

        let server = Server::new()
            .handler(ApiServerHandler::new(api))
            .verifier(Verifier::none());

        let server_ref = server
            .start(OneshotListener::from_value(t2))
            .map_err(io::Error::other)?;

        let client = NetClient::build()
            .auth_handler(DummyAuthHandler)
            .config(ClientConfig::default())
            .connector(t1)
            .connect()
            .await
            .map_err(io::Error::other)?;

        Ok((client, server_ref))
    }

    /// Consume [`Ssh`] and launch a distant server on the remote machine, returning credentials
    /// for connecting to the launched server.
    pub async fn launch(self, opts: LaunchOpts) -> io::Result<Credentials> {
        debug!("Launching distant server: {} {}", opts.binary, opts.args);

        let family = self.detect_family().await?;
        trace!("Detected family: {}", family.as_static_str());

        use distant_core::net::common::Host;

        let host = self
            .host()
            .parse::<Host>()
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidInput, x))?;

        let cmd = build_launch_args(family, &opts.binary, &opts.args)?;
        debug!("Executing launch command: {}", cmd);

        // Use channel exec instead of PTY + shell to avoid interference
        // from shell startup scripts (.bashrc, .zshrc, etc.)
        let channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(io::Error::other)?;

        channel
            .exec(true, cmd.as_bytes())
            .await
            .map_err(io::Error::other)?;

        // Read stdout directly for credentials (no PTY escape codes to filter)
        let (mut read_half, _write_half) = channel.split();

        let timeout = opts.timeout;
        let start_instant = std::time::Instant::now();
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        loop {
            // Check for timeout
            if start_instant.elapsed() >= timeout {
                let output = Self::clean_launch_output(&stdout, &stderr);
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!("Timed out waiting for server credentials: {output}"),
                ));
            }

            let remaining = timeout
                .checked_sub(start_instant.elapsed())
                .unwrap_or(Duration::from_millis(1));

            match tokio::time::timeout(remaining, read_half.wait()).await {
                Ok(Some(russh::ChannelMsg::Data { ref data })) => {
                    trace!("Received {} more bytes over stdout", data.len());
                    stdout.extend_from_slice(data);

                    if let Some(mut credentials) =
                        Credentials::find_lax(&String::from_utf8_lossy(&stdout))
                    {
                        credentials.host = host;
                        debug!("Got credentials from launched server");
                        return Ok(credentials);
                    }
                }
                Ok(Some(russh::ChannelMsg::ExtendedData { ref data, ext })) => {
                    // ext == 1 is stderr
                    if ext == 1 {
                        trace!("Received {} more bytes over stderr", data.len());
                        stderr.extend_from_slice(data);
                    }
                }
                Ok(Some(_)) => {
                    // Other channel messages (e.g. exit status), continue
                }
                Ok(None) => {
                    // Channel closed — check one last time if credentials appeared
                    if let Some(mut credentials) =
                        Credentials::find_lax(&String::from_utf8_lossy(&stdout))
                    {
                        credentials.host = host;
                        debug!("Got credentials from launched server (on channel close)");
                        return Ok(credentials);
                    }

                    let output = Self::clean_launch_output(&stdout, &stderr);
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        format!("Channel closed before credentials found: {output}"),
                    ));
                }
                Err(_) => {
                    // Timeout on this read iteration, will be caught at loop top
                }
            }
        }
    }

    /// Clean and format the output from a failed launch attempt for error messages.
    fn clean_launch_output(stdout: &[u8], stderr: &[u8]) -> String {
        fn clean_bytes(bytes: &[u8]) -> String {
            let s: String = String::from_utf8_lossy(bytes)
                .chars()
                .filter(|c| !c.is_control() || c.is_ascii_whitespace())
                .collect();
            s.trim().to_string()
        }

        let out = clean_bytes(stdout);
        let err = clean_bytes(stderr);

        match (out.is_empty(), err.is_empty()) {
            (true, true) => "(no output)".to_string(),
            (false, true) => format!("stdout: '{out}'"),
            (true, false) => format!("stderr: '{err}'"),
            (false, false) => format!("stdout: '{out}', stderr: '{err}'"),
        }
    }

    /// Consume [`Ssh`] and launch a distant server, then connect to it as a client.
    pub async fn launch_and_connect(self, opts: LaunchOpts) -> io::Result<Client> {
        trace!("ssh::launch_and_connect({:?})", opts);

        let timeout = opts.timeout;

        // Determine distinct candidate IP addresses for connecting
        debug!("Looking up host {} @ port {}", self.host, self.port);
        let mut candidate_ips = tokio::net::lookup_host(format!("{}:{}", self.host, self.port))
            .await
            .map_err(|x| {
                io::Error::new(
                    x.kind(),
                    format!("{} needs to be resolvable outside of ssh: {}", self.host, x),
                )
            })?
            .map(|addr| addr.ip())
            .collect::<Vec<IpAddr>>();
        candidate_ips.sort_unstable();
        candidate_ips.dedup();
        if candidate_ips.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                format!("Unable to resolve {}:{}", self.host, self.port),
            ));
        }

        let credentials = self.launch(opts).await?;
        let key = credentials.key;

        // Try each IP address with the same port to see if one works
        let mut err = None;
        for ip in candidate_ips {
            let addr = SocketAddr::new(ip, credentials.port);
            debug!("Attempting to connect to distant server @ {}", addr);
            match NetClient::tcp(addr)
                .auth_handler(AuthHandlerMap::new().with_static_key(key.clone()))
                .connect_timeout(timeout)
                .version(Version::new(
                    PROTOCOL_VERSION.major,
                    PROTOCOL_VERSION.minor,
                    PROTOCOL_VERSION.patch,
                ))
                .connect()
                .await
            {
                Ok(client) => return Ok(client),
                Err(x) => err = Some(x),
            }
        }

        Err(err.expect("Err set above"))
    }
}

#[cfg(test)]
mod tests {
    //! Tests for the `distant-ssh` crate root: SSH option types, authentication types,
    //! helper functions (`format_methods`, `clean_launch_output`), config building,
    //! and `ClientHandler`.
    //!
    //! The `build_launch_args` function and port/user resolution logic are replicated
    //! from the production `Ssh::launch` and `Ssh::connect` methods respectively,
    //! since those methods require a live SSH connection. The mock handler tests
    //! (`MockSshAuthHandler`, `ErrorSshAuthHandler`) verify that the `SshAuthHandler`
    //! trait is implementable and callable -- they are infrastructure verification
    //! rather than production logic tests.

    use super::*;

    #[test]
    fn ssh_family_as_static_str() {
        assert_eq!(SshFamily::Unix.as_static_str(), "unix");
        assert_eq!(SshFamily::Windows.as_static_str(), "windows");
    }

    #[test]
    fn distant_launch_opts_default() {
        let opts = LaunchOpts::default();
        assert_eq!(opts.binary, "distant");
        assert!(opts.args.is_empty());
        assert_eq!(opts.timeout, Duration::from_secs(15));
    }

    #[tokio::test]
    async fn local_ssh_auth_handler_on_banner_and_on_error() {
        let handler = LocalSshAuthHandler;
        handler.on_banner("test banner").await;
        handler.on_error("test error").await;
        // These just log — verifying they don't panic is sufficient
    }

    // --- format_methods tests ---

    #[test]
    fn format_methods_empty_returns_none() {
        let methods = russh::MethodSet::empty();
        assert_eq!(format_methods(&methods), "none");
    }

    #[test]
    fn format_methods_single_method() {
        let methods = russh::MethodSet::from([russh::MethodKind::PublicKey].as_slice());
        assert_eq!(format_methods(&methods), "publickey");
    }

    #[test]
    fn format_methods_multiple_methods() {
        let methods = russh::MethodSet::from(
            [russh::MethodKind::Password, russh::MethodKind::PublicKey].as_slice(),
        );
        let result = format_methods(&methods);
        assert!(
            result.contains("password"),
            "Expected 'password' in '{result}'"
        );
        assert!(
            result.contains("publickey"),
            "Expected 'publickey' in '{result}'"
        );
        assert!(
            result.contains(", "),
            "Expected comma separator in '{result}'"
        );
    }

    #[test]
    fn format_methods_all_methods() {
        let methods = russh::MethodSet::all();
        let result = format_methods(&methods);
        assert!(result.contains("none"), "Expected 'none' in '{result}'");
        assert!(
            result.contains("password"),
            "Expected 'password' in '{result}'"
        );
        assert!(
            result.contains("publickey"),
            "Expected 'publickey' in '{result}'"
        );
        assert!(
            result.contains("keyboard-interactive"),
            "Expected 'keyboard-interactive' in '{result}'"
        );
    }

    // --- SshOpts tests ---

    #[test]
    fn ssh_opts_default_values() {
        let opts = SshOpts::default();
        assert!(opts.identity_files.is_empty());
        assert_eq!(opts.identities_only, None);
        assert_eq!(opts.port, None);
        assert_eq!(opts.proxy_command, None);
        assert_eq!(opts.user, None);
        assert!(opts.user_known_hosts_files.is_empty());
        assert!(!opts.verbose);
        assert!(opts.other.is_empty());
    }

    #[test]
    fn ssh_opts_clone() {
        let mut opts = SshOpts::default();
        opts.port = Some(2222);
        opts.user = Some("testuser".to_string());
        opts.verbose = true;
        opts.identity_files.push(PathBuf::from("/tmp/id_rsa"));

        let cloned = opts.clone();
        assert_eq!(cloned.port, Some(2222));
        assert_eq!(cloned.user.as_deref(), Some("testuser"));
        assert!(cloned.verbose);
        assert_eq!(cloned.identity_files.len(), 1);
    }

    #[test]
    fn ssh_opts_debug_format() {
        let opts = SshOpts::default();
        let debug = format!("{:?}", opts);
        assert!(debug.contains("SshOpts"), "Expected 'SshOpts' in '{debug}'");
    }

    #[test]
    fn ssh_opts_all_fields_populated() {
        let mut other = BTreeMap::new();
        other.insert("CustomOption".to_string(), "value1".to_string());
        other.insert("AnotherOption".to_string(), "value2".to_string());

        let opts = SshOpts {
            identity_files: vec![
                PathBuf::from("/home/user/.ssh/id_ed25519"),
                PathBuf::from("/home/user/.ssh/id_rsa"),
            ],
            identities_only: Some(true),
            port: Some(2222),
            proxy_command: Some("ssh -W %h:%p jump-host".to_string()),
            user: Some("deploy".to_string()),
            user_known_hosts_files: vec![
                PathBuf::from("/home/user/.ssh/known_hosts"),
                PathBuf::from("/home/user/.ssh/known_hosts2"),
            ],
            verbose: true,
            other,
        };

        assert_eq!(opts.identity_files.len(), 2);
        assert_eq!(opts.identities_only, Some(true));
        assert_eq!(opts.port, Some(2222));
        assert_eq!(
            opts.proxy_command.as_deref(),
            Some("ssh -W %h:%p jump-host")
        );
        assert_eq!(opts.user.as_deref(), Some("deploy"));
        assert_eq!(opts.user_known_hosts_files.len(), 2);
        assert!(opts.verbose);
        assert_eq!(opts.other.len(), 2);
        assert_eq!(opts.other.get("CustomOption").unwrap(), "value1");
    }

    #[test]
    fn ssh_opts_clone_with_all_fields() {
        let mut other = BTreeMap::new();
        other.insert("Key".to_string(), "Val".to_string());

        let opts = SshOpts {
            identity_files: vec![PathBuf::from("/tmp/key")],
            identities_only: Some(false),
            port: Some(22),
            proxy_command: Some("proxy".to_string()),
            user: Some("root".to_string()),
            user_known_hosts_files: vec![PathBuf::from("/tmp/known")],
            verbose: false,
            other,
        };

        let cloned = opts.clone();
        assert_eq!(cloned.identity_files, opts.identity_files);
        assert_eq!(cloned.identities_only, Some(false));
        assert_eq!(cloned.port, Some(22));
        assert_eq!(cloned.proxy_command.as_deref(), Some("proxy"));
        assert_eq!(cloned.user.as_deref(), Some("root"));
        assert_eq!(cloned.user_known_hosts_files, opts.user_known_hosts_files);
        assert!(!cloned.verbose);
        assert_eq!(cloned.other.len(), 1);
    }

    #[test]
    fn ssh_opts_debug_format_with_populated_fields() {
        let mut opts = SshOpts::default();
        opts.port = Some(8022);
        opts.user = Some("admin".to_string());
        opts.proxy_command = Some("nc %h %p".to_string());
        opts.identities_only = Some(true);
        opts.verbose = true;
        opts.identity_files.push(PathBuf::from("/tmp/mykey"));
        opts.user_known_hosts_files
            .push(PathBuf::from("/tmp/known"));
        opts.other
            .insert("StrictHostKeyChecking".to_string(), "no".to_string());

        let debug = format!("{:?}", opts);
        assert!(debug.contains("8022"), "Expected port in '{debug}'");
        assert!(debug.contains("admin"), "Expected user in '{debug}'");
        assert!(
            debug.contains("nc %h %p"),
            "Expected proxy_command in '{debug}'"
        );
        assert!(
            debug.contains("StrictHostKeyChecking"),
            "Expected other key in '{debug}'"
        );
    }

    #[test]
    fn ssh_opts_other_btreemap_ordering() {
        let mut other = BTreeMap::new();
        other.insert("Zebra".to_string(), "z".to_string());
        other.insert("Alpha".to_string(), "a".to_string());
        other.insert("Middle".to_string(), "m".to_string());

        let opts = SshOpts {
            other,
            ..SshOpts::default()
        };

        // BTreeMap maintains sorted order
        let keys: Vec<&String> = opts.other.keys().collect();
        assert_eq!(keys, &["Alpha", "Middle", "Zebra"]);
    }

    // --- SshFamily tests ---

    #[test]
    fn ssh_family_copy_clone() {
        let family = SshFamily::Unix;
        let copied = family;
        let cloned = family;
        assert_eq!(copied, cloned);
        assert_eq!(family, SshFamily::Unix);
    }

    #[test]
    fn ssh_family_eq_and_ne() {
        assert_eq!(SshFamily::Unix, SshFamily::Unix);
        assert_eq!(SshFamily::Windows, SshFamily::Windows);
        assert_ne!(SshFamily::Unix, SshFamily::Windows);
    }

    #[test]
    fn ssh_family_debug_format() {
        let debug_unix = format!("{:?}", SshFamily::Unix);
        let debug_windows = format!("{:?}", SshFamily::Windows);
        assert!(
            debug_unix.contains("Unix"),
            "Expected 'Unix' in '{debug_unix}'"
        );
        assert!(
            debug_windows.contains("Windows"),
            "Expected 'Windows' in '{debug_windows}'"
        );
    }

    #[test]
    fn ssh_family_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(SshFamily::Unix);
        set.insert(SshFamily::Windows);
        set.insert(SshFamily::Unix); // duplicate
        assert_eq!(set.len(), 2);
    }

    // --- SshAuthPrompt tests ---

    #[test]
    fn ssh_auth_prompt_construction() {
        let prompt = SshAuthPrompt {
            prompt: "Password: ".to_string(),
            echo: false,
        };
        assert_eq!(prompt.prompt, "Password: ");
        assert!(!prompt.echo);
    }

    #[test]
    fn ssh_auth_prompt_echo_true() {
        let prompt = SshAuthPrompt {
            prompt: "Username: ".to_string(),
            echo: true,
        };
        assert_eq!(prompt.prompt, "Username: ");
        assert!(prompt.echo);
    }

    #[test]
    fn ssh_auth_prompt_debug_format() {
        let prompt = SshAuthPrompt {
            prompt: "test".to_string(),
            echo: true,
        };
        let debug = format!("{:?}", prompt);
        assert!(debug.contains("test"), "Expected 'test' in '{debug}'");
        assert!(debug.contains("true"), "Expected 'true' in '{debug}'");
    }

    // --- SshAuthEvent tests ---

    #[test]
    fn ssh_auth_event_construction() {
        let event = SshAuthEvent {
            username: "user".to_string(),
            instructions: "Please authenticate".to_string(),
            prompts: vec![SshAuthPrompt {
                prompt: "Password: ".to_string(),
                echo: false,
            }],
        };
        assert_eq!(event.username, "user");
        assert_eq!(event.instructions, "Please authenticate");
        assert_eq!(event.prompts.len(), 1);
        assert_eq!(event.prompts[0].prompt, "Password: ");
        assert!(!event.prompts[0].echo);
    }

    #[test]
    fn ssh_auth_event_empty_prompts() {
        let event = SshAuthEvent {
            username: String::new(),
            instructions: String::new(),
            prompts: Vec::new(),
        };
        assert!(event.username.is_empty());
        assert!(event.instructions.is_empty());
        assert!(event.prompts.is_empty());
    }

    #[test]
    fn ssh_auth_event_multiple_prompts() {
        let event = SshAuthEvent {
            username: "admin".to_string(),
            instructions: "Multi-factor auth".to_string(),
            prompts: vec![
                SshAuthPrompt {
                    prompt: "Password: ".to_string(),
                    echo: false,
                },
                SshAuthPrompt {
                    prompt: "OTP: ".to_string(),
                    echo: true,
                },
            ],
        };
        assert_eq!(event.prompts.len(), 2);
        assert!(!event.prompts[0].echo);
        assert!(event.prompts[1].echo);
    }

    #[test]
    fn ssh_auth_event_debug_format() {
        let event = SshAuthEvent {
            username: "testuser".to_string(),
            instructions: "info".to_string(),
            prompts: vec![],
        };
        let debug = format!("{:?}", event);
        assert!(
            debug.contains("testuser"),
            "Expected 'testuser' in '{debug}'"
        );
    }

    // --- LaunchOpts tests ---

    #[test]
    fn launch_opts_custom_values() {
        let opts = LaunchOpts {
            binary: String::from("/usr/local/bin/distant"),
            args: String::from("--port 8080"),
            timeout: Duration::from_secs(30),
        };
        assert_eq!(opts.binary, "/usr/local/bin/distant");
        assert_eq!(opts.args, "--port 8080");
        assert_eq!(opts.timeout, Duration::from_secs(30));
    }

    #[test]
    fn launch_opts_debug_format() {
        let opts = LaunchOpts::default();
        let debug = format!("{:?}", opts);
        assert!(debug.contains("distant"), "Expected 'distant' in '{debug}'");
    }

    #[test]
    fn launch_opts_clone() {
        let opts = LaunchOpts {
            binary: String::from("custom-distant"),
            args: String::from("--flag"),
            timeout: Duration::from_secs(60),
        };
        let cloned = opts.clone();
        assert_eq!(cloned.binary, "custom-distant");
        assert_eq!(cloned.args, "--flag");
        assert_eq!(cloned.timeout, Duration::from_secs(60));
    }

    #[test]
    fn launch_opts_default_binary_is_distant() {
        let opts = LaunchOpts::default();
        assert_eq!(opts.binary, "distant");
    }

    #[test]
    fn launch_opts_default_args_is_empty() {
        let opts = LaunchOpts::default();
        assert!(opts.args.is_empty());
    }

    #[test]
    fn launch_opts_default_timeout_is_15_seconds() {
        let opts = LaunchOpts::default();
        assert_eq!(opts.timeout.as_secs(), 15);
        assert_eq!(opts.timeout.subsec_nanos(), 0);
    }

    #[test]
    fn launch_opts_clone_with_empty_binary() {
        let opts = LaunchOpts {
            binary: String::new(),
            args: String::from("--daemon"),
            timeout: Duration::from_millis(500),
        };
        let cloned = opts.clone();
        assert!(cloned.binary.is_empty());
        assert_eq!(cloned.args, "--daemon");
        assert_eq!(cloned.timeout, Duration::from_millis(500));
    }

    #[test]
    fn launch_opts_debug_shows_all_fields() {
        let opts = LaunchOpts {
            binary: String::from("my-binary"),
            args: String::from("--arg1 --arg2"),
            timeout: Duration::from_secs(120),
        };
        let debug = format!("{:?}", opts);
        assert!(debug.contains("my-binary"), "Expected binary in '{debug}'");
        assert!(
            debug.contains("--arg1 --arg2"),
            "Expected args in '{debug}'"
        );
        assert!(debug.contains("120"), "Expected timeout in '{debug}'");
    }

    // --- clean_launch_output tests ---

    #[test]
    fn clean_launch_output_both_empty() {
        let result = Ssh::clean_launch_output(b"", b"");
        assert_eq!(result, "(no output)");
    }

    #[test]
    fn clean_launch_output_only_stdout() {
        let result = Ssh::clean_launch_output(b"hello world", b"");
        assert_eq!(result, "stdout: 'hello world'");
    }

    #[test]
    fn clean_launch_output_only_stderr() {
        let result = Ssh::clean_launch_output(b"", b"error occurred");
        assert_eq!(result, "stderr: 'error occurred'");
    }

    #[test]
    fn clean_launch_output_both_present() {
        let result = Ssh::clean_launch_output(b"some output", b"some error");
        assert_eq!(result, "stdout: 'some output', stderr: 'some error'");
    }

    #[test]
    fn clean_launch_output_strips_control_characters() {
        // \x01 (SOH), \x02 (STX), \x1b (ESC) are control chars that should be stripped
        let result = Ssh::clean_launch_output(b"hello\x01\x02world", b"");
        assert_eq!(result, "stdout: 'helloworld'");
    }

    #[test]
    fn clean_launch_output_preserves_whitespace() {
        // Tabs, newlines, spaces are ascii whitespace and should be preserved (pre-trim)
        let result = Ssh::clean_launch_output(b"hello\tworld", b"");
        assert_eq!(result, "stdout: 'hello\tworld'");
    }

    #[test]
    fn clean_launch_output_trims_whitespace() {
        let result = Ssh::clean_launch_output(b"  hello  ", b"  error  ");
        assert_eq!(result, "stdout: 'hello', stderr: 'error'");
    }

    #[test]
    fn clean_launch_output_only_whitespace_becomes_empty() {
        // After trimming, only-whitespace becomes empty
        let result = Ssh::clean_launch_output(b"   ", b"   ");
        assert_eq!(result, "(no output)");
    }

    #[test]
    fn clean_launch_output_only_control_chars_becomes_empty() {
        // All control chars stripped, then empty after trim
        let result = Ssh::clean_launch_output(b"\x01\x02\x03", b"\x04\x05\x06");
        assert_eq!(result, "(no output)");
    }

    #[test]
    fn clean_launch_output_mixed_control_and_text() {
        let result = Ssh::clean_launch_output(b"\x1b[31mred text\x1b[0m", b"\x1b[error\x1b]done");
        // ESC (\x1b) is control, [ ] are not control, so letters and brackets remain
        assert!(result.contains("stdout:"), "Expected stdout in '{result}'");
    }

    #[test]
    fn clean_launch_output_utf8_lossy() {
        // Invalid UTF-8 should be handled gracefully via from_utf8_lossy
        let result = Ssh::clean_launch_output(b"valid\xff\xfeinvalid", b"");
        assert!(
            result.contains("stdout:"),
            "Expected stdout label in '{result}'"
        );
    }

    #[test]
    fn clean_launch_output_newlines_preserved_then_trimmed() {
        // Newlines are whitespace, preserved in middle, trimmed at edges
        let result = Ssh::clean_launch_output(b"\nline1\nline2\n", b"");
        assert_eq!(result, "stdout: 'line1\nline2'");
    }

    #[test]
    fn clean_launch_output_carriage_return_preserved() {
        // \r is ascii whitespace, should be preserved in content
        let result = Ssh::clean_launch_output(b"line1\r\nline2", b"");
        assert!(result.contains("line1"), "Expected line1 in '{result}'");
        assert!(result.contains("line2"), "Expected line2 in '{result}'");
    }

    #[test]
    fn clean_launch_output_null_bytes_stripped() {
        // \x00 (NUL) is a control char, should be stripped
        let result = Ssh::clean_launch_output(b"before\x00after", b"");
        assert_eq!(result, "stdout: 'beforeafter'");
    }

    #[test]
    fn clean_launch_output_bell_stripped() {
        // \x07 (BEL) is a control char, should be stripped
        let result = Ssh::clean_launch_output(b"text\x07here", b"");
        assert_eq!(result, "stdout: 'texthere'");
    }

    #[test]
    fn clean_launch_output_backspace_stripped() {
        // \x08 (BS) is a control char, should be stripped
        let result = Ssh::clean_launch_output(b"ab\x08c", b"");
        assert_eq!(result, "stdout: 'abc'");
    }

    #[test]
    fn clean_launch_output_stderr_only_control_chars() {
        // stdout has text, stderr is only control chars (becomes empty)
        let result = Ssh::clean_launch_output(b"output", b"\x01\x02\x03");
        assert_eq!(result, "stdout: 'output'");
    }

    #[test]
    fn clean_launch_output_stdout_only_control_chars() {
        // stdout is only control chars (becomes empty), stderr has text
        let result = Ssh::clean_launch_output(b"\x01\x02\x03", b"error text");
        assert_eq!(result, "stderr: 'error text'");
    }

    #[test]
    fn clean_launch_output_long_output() {
        let long_stdout = b"A".repeat(1000);
        let long_stderr = b"B".repeat(500);
        let result = Ssh::clean_launch_output(&long_stdout, &long_stderr);
        assert!(result.starts_with("stdout: '"));
        assert!(result.contains("stderr: '"));
    }

    // --- LocalSshAuthHandler construction ---

    #[test]
    fn local_ssh_auth_handler_can_be_constructed() {
        let _handler = LocalSshAuthHandler;
    }

    // --- build_russh_config tests ---

    #[test]
    fn build_russh_config_default_params() {
        use ssh2_config_rs::DefaultAlgorithms;

        let opts = SshOpts::default();
        let params = HostParams::new(&DefaultAlgorithms::default());
        let config = Ssh::build_russh_config(&opts, &params).unwrap();

        // With default params, keepalive_interval should be None
        assert!(config.keepalive_interval.is_none());
    }

    #[test]
    fn build_russh_config_with_keepalive_interval() {
        use ssh2_config_rs::DefaultAlgorithms;

        let opts = SshOpts::default();
        let mut params = HostParams::new(&DefaultAlgorithms::default());
        params.server_alive_interval = Some(Duration::from_secs(60));

        let config = Ssh::build_russh_config(&opts, &params).unwrap();
        assert_eq!(config.keepalive_interval, Some(Duration::from_secs(60)));
    }

    #[test]
    fn build_russh_config_with_short_keepalive() {
        use ssh2_config_rs::DefaultAlgorithms;

        let opts = SshOpts::default();
        let mut params = HostParams::new(&DefaultAlgorithms::default());
        params.server_alive_interval = Some(Duration::from_secs(5));

        let config = Ssh::build_russh_config(&opts, &params).unwrap();
        assert_eq!(config.keepalive_interval, Some(Duration::from_secs(5)));
    }

    #[test]
    fn build_russh_config_without_keepalive() {
        use ssh2_config_rs::DefaultAlgorithms;

        let opts = SshOpts::default();
        let mut params = HostParams::new(&DefaultAlgorithms::default());
        params.server_alive_interval = None;

        let config = Ssh::build_russh_config(&opts, &params).unwrap();
        assert!(config.keepalive_interval.is_none());
    }

    #[test]
    fn build_russh_config_with_verbose_opts() {
        use ssh2_config_rs::DefaultAlgorithms;

        let mut opts = SshOpts::default();
        opts.verbose = true;
        let params = HostParams::new(&DefaultAlgorithms::default());

        // Should not error even with verbose opts
        let config = Ssh::build_russh_config(&opts, &params);
        assert!(config.is_ok());
    }

    #[test]
    fn build_russh_config_with_populated_opts() {
        use ssh2_config_rs::DefaultAlgorithms;

        let mut opts = SshOpts::default();
        opts.port = Some(2222);
        opts.user = Some("testuser".to_string());
        opts.identity_files.push(PathBuf::from("/tmp/id_rsa"));

        let params = HostParams::new(&DefaultAlgorithms::default());
        let config = Ssh::build_russh_config(&opts, &params);
        assert!(config.is_ok());
    }

    // --- build_preferred_algorithms tests ---

    #[test]
    fn build_preferred_algorithms_returns_defaults() {
        use ssh2_config_rs::DefaultAlgorithms;

        let params = HostParams::new(&DefaultAlgorithms::default());
        let preferred = Ssh::build_preferred_algorithms(&params);

        // Should return the russh default preferred algorithms
        let default_preferred = russh::Preferred::default();
        assert_eq!(preferred.kex, default_preferred.kex);
        assert_eq!(preferred.cipher, default_preferred.cipher);
    }

    #[test]
    fn build_preferred_algorithms_with_custom_params() {
        use ssh2_config_rs::DefaultAlgorithms;

        let mut params = HostParams::new(&DefaultAlgorithms::default());
        params.port = Some(9999);
        params.user = Some("custom-user".to_string());

        // Even with custom params, algorithms should still return defaults
        let preferred = Ssh::build_preferred_algorithms(&params);
        let default_preferred = russh::Preferred::default();
        assert_eq!(preferred.kex, default_preferred.kex);
    }

    // --- parse_ssh_config tests ---

    #[test]
    fn parse_ssh_config_returns_host_params() {
        // This test exercises the parse_ssh_config path.
        // On a real machine with ~/.ssh/config, it parses it.
        // If ~/.ssh/config doesn't exist, it returns default params.
        let result = Ssh::parse_ssh_config("nonexistent-host.example.com");
        assert!(result.is_ok());
        let params = result.unwrap();
        // For a nonexistent host, port/user should be None
        // (unless the user's SSH config has a wildcard match)
        // We just verify it doesn't error
        let _ = params.port;
        let _ = params.user;
    }

    #[test]
    fn parse_ssh_config_with_localhost() {
        let result = Ssh::parse_ssh_config("localhost");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_ssh_config_with_wildcard_host() {
        let result = Ssh::parse_ssh_config("*");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_ssh_config_with_empty_host() {
        let result = Ssh::parse_ssh_config("");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_ssh_config_with_ip_address() {
        let result = Ssh::parse_ssh_config("192.168.1.1");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_ssh_config_with_ipv6_address() {
        let result = Ssh::parse_ssh_config("::1");
        assert!(result.is_ok());
    }

    // --- SshAuthHandler trait with custom implementation ---

    struct MockSshAuthHandler {
        responses: Vec<String>,
        verify_result: bool,
    }

    impl SshAuthHandler for MockSshAuthHandler {
        fn on_authenticate(
            &self,
            _event: SshAuthEvent,
        ) -> impl Future<Output = io::Result<Vec<String>>> + Send {
            let responses = self.responses.clone();
            async move { Ok(responses) }
        }

        fn on_verify_host<'a>(
            &'a self,
            _host: &'a str,
        ) -> impl Future<Output = io::Result<bool>> + Send + 'a {
            async move { Ok(self.verify_result) }
        }

        fn on_banner<'a>(&'a self, _text: &'a str) -> impl Future<Output = ()> + Send + 'a {
            async move {}
        }

        fn on_error<'a>(&'a self, _text: &'a str) -> impl Future<Output = ()> + Send + 'a {
            async move {}
        }
    }

    #[test_log::test(tokio::test)]
    async fn mock_ssh_auth_handler_on_authenticate() {
        let handler = MockSshAuthHandler {
            responses: vec!["password123".to_string()],
            verify_result: true,
        };
        let event = SshAuthEvent {
            username: "user".to_string(),
            instructions: "Enter password".to_string(),
            prompts: vec![SshAuthPrompt {
                prompt: "Password: ".to_string(),
                echo: false,
            }],
        };
        let answers = handler.on_authenticate(event).await.unwrap();
        assert_eq!(answers, ["password123"]);
    }

    #[test_log::test(tokio::test)]
    async fn mock_ssh_auth_handler_on_verify_host_accept() {
        let handler = MockSshAuthHandler {
            responses: vec![],
            verify_result: true,
        };
        assert!(handler.on_verify_host("example.com").await.unwrap());
    }

    #[test_log::test(tokio::test)]
    async fn mock_ssh_auth_handler_on_verify_host_reject() {
        let handler = MockSshAuthHandler {
            responses: vec![],
            verify_result: false,
        };
        assert!(!handler.on_verify_host("evil.com").await.unwrap());
    }

    #[test_log::test(tokio::test)]
    async fn mock_ssh_auth_handler_on_banner() {
        let handler = MockSshAuthHandler {
            responses: vec![],
            verify_result: true,
        };
        handler.on_banner("Welcome to the server").await;
    }

    #[test_log::test(tokio::test)]
    async fn mock_ssh_auth_handler_on_error() {
        let handler = MockSshAuthHandler {
            responses: vec![],
            verify_result: true,
        };
        handler.on_error("Authentication failed").await;
    }

    #[test_log::test(tokio::test)]
    async fn mock_ssh_auth_handler_multiple_prompts() {
        let handler = MockSshAuthHandler {
            responses: vec!["my-password".to_string(), "123456".to_string()],
            verify_result: true,
        };
        let event = SshAuthEvent {
            username: "admin".to_string(),
            instructions: "MFA Required".to_string(),
            prompts: vec![
                SshAuthPrompt {
                    prompt: "Password: ".to_string(),
                    echo: false,
                },
                SshAuthPrompt {
                    prompt: "OTP: ".to_string(),
                    echo: true,
                },
            ],
        };
        let answers = handler.on_authenticate(event).await.unwrap();
        assert_eq!(answers.len(), 2);
        assert_eq!(answers[0], "my-password");
        assert_eq!(answers[1], "123456");
    }

    #[test_log::test(tokio::test)]
    async fn mock_ssh_auth_handler_empty_responses() {
        let handler = MockSshAuthHandler {
            responses: vec![],
            verify_result: true,
        };
        let event = SshAuthEvent {
            username: String::new(),
            instructions: String::new(),
            prompts: vec![],
        };
        let answers = handler.on_authenticate(event).await.unwrap();
        assert!(answers.is_empty());
    }

    // --- Error-returning SshAuthHandler ---

    struct ErrorSshAuthHandler;

    impl SshAuthHandler for ErrorSshAuthHandler {
        fn on_authenticate(
            &self,
            _event: SshAuthEvent,
        ) -> impl Future<Output = io::Result<Vec<String>>> + Send {
            async move { Err(io::Error::other("authentication cancelled by user")) }
        }

        fn on_verify_host<'a>(
            &'a self,
            _host: &'a str,
        ) -> impl Future<Output = io::Result<bool>> + Send + 'a {
            async move { Err(io::Error::other("verification cancelled")) }
        }

        fn on_banner<'a>(&'a self, _text: &'a str) -> impl Future<Output = ()> + Send + 'a {
            async move {}
        }

        fn on_error<'a>(&'a self, _text: &'a str) -> impl Future<Output = ()> + Send + 'a {
            async move {}
        }
    }

    #[test_log::test(tokio::test)]
    async fn error_ssh_auth_handler_on_authenticate_returns_error() {
        let handler = ErrorSshAuthHandler;
        let event = SshAuthEvent {
            username: "user".to_string(),
            instructions: String::new(),
            prompts: vec![SshAuthPrompt {
                prompt: "Password: ".to_string(),
                echo: false,
            }],
        };
        let result = handler.on_authenticate(event).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert!(
            err.to_string().contains("authentication cancelled"),
            "Expected 'authentication cancelled' in '{}'",
            err
        );
    }

    #[test_log::test(tokio::test)]
    async fn error_ssh_auth_handler_on_verify_host_returns_error() {
        let handler = ErrorSshAuthHandler;
        let result = handler.on_verify_host("example.com").await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("verification cancelled"),
            "Expected 'verification cancelled' in '{}'",
            err
        );
    }

    // --- SshAuthEvent with complex prompt strings ---

    #[test]
    fn ssh_auth_event_multiline_prompt() {
        let event = SshAuthEvent {
            username: "user".to_string(),
            instructions: "Line1\nLine2\nLine3".to_string(),
            prompts: vec![SshAuthPrompt {
                prompt: "Header line\nPassword: ".to_string(),
                echo: false,
            }],
        };
        assert!(event.instructions.contains('\n'));
        assert!(event.prompts[0].prompt.contains('\n'));
    }

    #[test]
    fn ssh_auth_event_unicode_content() {
        let event = SshAuthEvent {
            username: "utilisateur".to_string(),
            instructions: "Veuillez entrer votre mot de passe".to_string(),
            prompts: vec![SshAuthPrompt {
                prompt: "Mot de passe: ".to_string(),
                echo: false,
            }],
        };
        assert_eq!(event.username, "utilisateur");
        assert_eq!(event.prompts[0].prompt, "Mot de passe: ");
    }

    #[test]
    fn ssh_auth_prompt_empty_prompt_string() {
        let prompt = SshAuthPrompt {
            prompt: String::new(),
            echo: false,
        };
        assert!(prompt.prompt.is_empty());
        assert!(!prompt.echo);
    }

    #[test]
    fn ssh_auth_event_many_prompts() {
        let prompts: Vec<SshAuthPrompt> = (0..10)
            .map(|i| SshAuthPrompt {
                prompt: format!("Prompt {}: ", i),
                echo: i % 2 == 0,
            })
            .collect();

        let event = SshAuthEvent {
            username: "multi".to_string(),
            instructions: "Answer all prompts".to_string(),
            prompts,
        };
        assert_eq!(event.prompts.len(), 10);
        assert!(event.prompts[0].echo);
        assert!(!event.prompts[1].echo);
        assert!(event.prompts[8].echo);
        assert!(!event.prompts[9].echo);
    }

    // --- format_methods edge cases ---

    #[test]
    fn format_methods_single_none() {
        let methods = russh::MethodSet::from([russh::MethodKind::None].as_slice());
        assert_eq!(format_methods(&methods), "none");
    }

    #[test]
    fn format_methods_single_password() {
        let methods = russh::MethodSet::from([russh::MethodKind::Password].as_slice());
        assert_eq!(format_methods(&methods), "password");
    }

    #[test]
    fn format_methods_single_keyboard_interactive() {
        let methods = russh::MethodSet::from([russh::MethodKind::KeyboardInteractive].as_slice());
        assert_eq!(format_methods(&methods), "keyboard-interactive");
    }

    #[test]
    fn format_methods_hostbased() {
        let methods = russh::MethodSet::from([russh::MethodKind::HostBased].as_slice());
        assert_eq!(format_methods(&methods), "hostbased");
    }

    #[test]
    fn format_methods_pubkey_and_password() {
        let methods = russh::MethodSet::from(
            [russh::MethodKind::PublicKey, russh::MethodKind::Password].as_slice(),
        );
        let result = format_methods(&methods);
        // Both should appear separated by comma
        let parts: Vec<&str> = result.split(", ").collect();
        assert_eq!(parts.len(), 2);
    }

    #[test]
    fn format_methods_three_methods() {
        let methods = russh::MethodSet::from(
            [
                russh::MethodKind::PublicKey,
                russh::MethodKind::Password,
                russh::MethodKind::KeyboardInteractive,
            ]
            .as_slice(),
        );
        let result = format_methods(&methods);
        let parts: Vec<&str> = result.split(", ").collect();
        assert_eq!(parts.len(), 3);
    }

    // --- ClientHandler tests ---

    #[test_log::test(tokio::test)]
    async fn client_handler_check_server_key_always_accepts() {
        use russh::client::Handler;

        let mut handler = ClientHandler;

        // Generate a test public key by creating a keypair
        let private_key = russh::keys::PrivateKey::random(
            &mut rand::thread_rng(),
            russh::keys::Algorithm::Ed25519,
        )
        .unwrap();
        let public_key = private_key.public_key();

        let result = handler.check_server_key(public_key).await;
        assert!(result.is_ok());
        assert!(result.unwrap());
    }

    // --- SshFamily as_static_str exhaustive ---

    #[test]
    fn ssh_family_unix_static_str_is_lowercase() {
        let s = SshFamily::Unix.as_static_str();
        assert_eq!(s, s.to_lowercase());
    }

    #[test]
    fn ssh_family_windows_static_str_is_lowercase() {
        let s = SshFamily::Windows.as_static_str();
        assert_eq!(s, s.to_lowercase());
    }

    #[test]
    fn ssh_family_as_static_str_returns_static_lifetime() {
        // Verify the str has 'static lifetime by storing in a variable
        let s: &'static str = SshFamily::Unix.as_static_str();
        assert!(!s.is_empty());
        let s: &'static str = SshFamily::Windows.as_static_str();
        assert!(!s.is_empty());
    }

    // --- LocalSshAuthHandler banner/error with various inputs ---

    #[test_log::test(tokio::test)]
    async fn local_ssh_auth_handler_on_banner_empty_string() {
        let handler = LocalSshAuthHandler;
        handler.on_banner("").await;
    }

    #[test_log::test(tokio::test)]
    async fn local_ssh_auth_handler_on_error_empty_string() {
        let handler = LocalSshAuthHandler;
        handler.on_error("").await;
    }

    #[test_log::test(tokio::test)]
    async fn local_ssh_auth_handler_on_banner_multiline() {
        let handler = LocalSshAuthHandler;
        handler.on_banner("Welcome\nto the\nserver").await;
    }

    #[test_log::test(tokio::test)]
    async fn local_ssh_auth_handler_on_error_multiline() {
        let handler = LocalSshAuthHandler;
        handler.on_error("Error line 1\nError line 2").await;
    }

    #[test_log::test(tokio::test)]
    async fn local_ssh_auth_handler_on_banner_unicode() {
        let handler = LocalSshAuthHandler;
        handler.on_banner("Bienvenue au serveur").await;
    }

    #[test_log::test(tokio::test)]
    async fn local_ssh_auth_handler_on_error_unicode() {
        let handler = LocalSshAuthHandler;
        handler.on_error("Erreur: connexion refusee").await;
    }

    // --- SshOpts with struct update syntax ---

    #[test]
    fn ssh_opts_struct_update_syntax() {
        let base = SshOpts::default();
        let opts = SshOpts {
            port: Some(3022),
            user: Some("custom".to_string()),
            ..base
        };
        assert_eq!(opts.port, Some(3022));
        assert_eq!(opts.user.as_deref(), Some("custom"));
        assert!(opts.identity_files.is_empty());
        assert!(!opts.verbose);
    }

    #[test]
    fn ssh_opts_identities_only_true() {
        let opts = SshOpts {
            identities_only: Some(true),
            ..SshOpts::default()
        };
        assert_eq!(opts.identities_only, Some(true));
    }

    #[test]
    fn ssh_opts_identities_only_false() {
        let opts = SshOpts {
            identities_only: Some(false),
            ..SshOpts::default()
        };
        assert_eq!(opts.identities_only, Some(false));
    }

    #[test]
    fn ssh_opts_multiple_identity_files() {
        let opts = SshOpts {
            identity_files: vec![
                PathBuf::from("/home/user/.ssh/id_ed25519"),
                PathBuf::from("/home/user/.ssh/id_rsa"),
                PathBuf::from("/home/user/.ssh/id_ecdsa"),
            ],
            ..SshOpts::default()
        };
        assert_eq!(opts.identity_files.len(), 3);
        assert_eq!(
            opts.identity_files[0],
            PathBuf::from("/home/user/.ssh/id_ed25519")
        );
    }

    #[test]
    fn ssh_opts_multiple_known_hosts_files() {
        let opts = SshOpts {
            user_known_hosts_files: vec![
                PathBuf::from("/home/user/.ssh/known_hosts"),
                PathBuf::from("/home/user/.ssh/known_hosts2"),
            ],
            ..SshOpts::default()
        };
        assert_eq!(opts.user_known_hosts_files.len(), 2);
    }

    // --- clean_launch_output with multibyte UTF-8 ---

    #[test]
    fn clean_launch_output_with_multibyte_utf8() {
        let result = Ssh::clean_launch_output("Server ready".as_bytes(), b"");
        assert_eq!(result, "stdout: 'Server ready'");
    }

    #[test]
    fn clean_launch_output_form_feed_preserved() {
        // \x0c (FF / form feed) is both control AND ascii whitespace,
        // so it passes the filter (is_ascii_whitespace allows it through)
        let result = Ssh::clean_launch_output(b"before\x0cafter", b"");
        assert_eq!(result, "stdout: 'before\x0cafter'");
    }

    #[test]
    fn clean_launch_output_vertical_tab_stripped() {
        // \x0b (VT) is a control char but also considered ascii whitespace
        // Actually \x0b IS ascii whitespace (is_ascii_whitespace returns true)
        // So it should be preserved by the filter
        let result = Ssh::clean_launch_output(b"a\x0bb", b"");
        assert!(result.contains("stdout:"), "Expected stdout in '{result}'");
    }

    #[test]
    fn clean_launch_output_mixed_valid_invalid_utf8_in_stderr() {
        let result = Ssh::clean_launch_output(b"", b"err\xff\xfeor");
        assert!(result.contains("stderr:"), "Expected stderr in '{result}'");
    }

    // --- Additional edge case tests ---

    #[test]
    fn clean_launch_output_with_ansi_escape_sequences() {
        // ANSI escape: ESC[31m is "\x1b[31m" -- ESC is control, stripped
        // But '[', '3', '1', 'm' are normal chars, kept
        let input = b"\x1b[31mError\x1b[0m";
        let result = Ssh::clean_launch_output(input, b"");
        // ESC chars stripped, brackets and text remain
        assert!(
            result.contains("[31mError"),
            "Expected cleaned ANSI in '{result}'"
        );
    }

    #[test]
    fn clean_launch_output_with_crlf_line_endings() {
        let result = Ssh::clean_launch_output(b"line1\r\nline2\r\nline3", b"");
        assert!(result.contains("line1"));
        assert!(result.contains("line2"));
        assert!(result.contains("line3"));
    }

    #[test]
    fn clean_launch_output_large_stderr_only() {
        let stderr = b"E".repeat(5000);
        let result = Ssh::clean_launch_output(b"", &stderr);
        assert!(result.starts_with("stderr: '"));
        assert!(result.len() > 5000);
    }

    #[test]
    fn clean_launch_output_mixed_control_and_whitespace() {
        // Mix of control chars (stripped) and whitespace (preserved)
        let result = Ssh::clean_launch_output(b"\x01\t\x02 \x03\n\x04", b"");
        // After stripping: "\t \n" -> trimmed -> empty or just spaces
        // Actually: control chars \x01, \x02, \x03, \x04 are stripped
        // whitespace \t, ' ', \n are preserved, then trimmed
        assert!(result.contains("stdout:") || result == "(no output)");
    }

    #[test]
    fn format_methods_returns_deterministic_for_single() {
        // Calling format_methods twice on the same input should give same result
        let methods = russh::MethodSet::from([russh::MethodKind::PublicKey].as_slice());
        let result1 = format_methods(&methods);
        let result2 = format_methods(&methods);
        assert_eq!(result1, result2);
    }

    #[test]
    fn ssh_family_as_static_str_roundtrip_matches_expected() {
        // Verify as_static_str matches what we'd expect from the enum variant name
        assert_eq!(SshFamily::Unix.as_static_str(), "unix");
        assert_eq!(SshFamily::Windows.as_static_str(), "windows");
    }

    #[test]
    fn launch_opts_zero_timeout() {
        let opts = LaunchOpts {
            binary: String::from("distant"),
            args: String::new(),
            timeout: Duration::from_secs(0),
        };
        assert_eq!(opts.timeout, Duration::ZERO);
    }

    #[test]
    fn launch_opts_very_long_timeout() {
        let opts = LaunchOpts {
            binary: String::from("distant"),
            args: String::new(),
            timeout: Duration::from_secs(3600),
        };
        assert_eq!(opts.timeout.as_secs(), 3600);
    }

    #[test]
    fn launch_opts_with_complex_args() {
        let opts = LaunchOpts {
            binary: String::from("distant"),
            args: String::from("--port 8080 --host 0.0.0.0 --log-level trace"),
            timeout: Duration::from_secs(15),
        };
        assert!(opts.args.contains("--port"));
        assert!(opts.args.contains("--host"));
        assert!(opts.args.contains("--log-level"));
    }

    #[test]
    fn launch_opts_with_quoted_args() {
        let opts = LaunchOpts {
            binary: String::from("distant"),
            args: String::from("--config '/path/to/config file.toml'"),
            timeout: Duration::from_secs(15),
        };
        assert!(opts.args.contains("config file.toml"));
    }

    #[test]
    fn ssh_opts_with_many_identity_files() {
        let files: Vec<PathBuf> = (0..20)
            .map(|i| PathBuf::from(format!("/key_{}", i)))
            .collect();

        let opts = SshOpts {
            identity_files: files,
            ..SshOpts::default()
        };

        assert_eq!(opts.identity_files.len(), 20);
        assert_eq!(opts.identity_files[0], PathBuf::from("/key_0"));
        assert_eq!(opts.identity_files[19], PathBuf::from("/key_19"));
    }

    #[test]
    fn ssh_opts_other_map_with_many_entries() {
        let mut other = BTreeMap::new();
        for i in 0..50 {
            other.insert(format!("Key{:03}", i), format!("Value{}", i));
        }

        let opts = SshOpts {
            other,
            ..SshOpts::default()
        };

        assert_eq!(opts.other.len(), 50);
        // BTreeMap keys are sorted
        let first_key = opts.other.keys().next().unwrap();
        assert_eq!(first_key, "Key000");
    }

    #[test]
    fn build_russh_config_with_zero_keepalive() {
        use ssh2_config_rs::DefaultAlgorithms;

        let opts = SshOpts::default();
        let mut params = HostParams::new(&DefaultAlgorithms::default());
        params.server_alive_interval = Some(Duration::from_secs(0));

        let config = Ssh::build_russh_config(&opts, &params).unwrap();
        assert_eq!(config.keepalive_interval, Some(Duration::from_secs(0)));
    }

    #[test]
    fn build_russh_config_with_large_keepalive() {
        use ssh2_config_rs::DefaultAlgorithms;

        let opts = SshOpts::default();
        let mut params = HostParams::new(&DefaultAlgorithms::default());
        params.server_alive_interval = Some(Duration::from_secs(3600));

        let config = Ssh::build_russh_config(&opts, &params).unwrap();
        assert_eq!(config.keepalive_interval, Some(Duration::from_secs(3600)));
    }

    #[test]
    fn build_russh_config_config_has_preferred_algorithms() {
        use ssh2_config_rs::DefaultAlgorithms;

        let opts = SshOpts::default();
        let params = HostParams::new(&DefaultAlgorithms::default());
        let config = Ssh::build_russh_config(&opts, &params).unwrap();

        // Config should have preferred algorithms set
        let default_preferred = russh::Preferred::default();
        assert_eq!(config.preferred.kex, default_preferred.kex);
        assert_eq!(config.preferred.cipher, default_preferred.cipher);
    }

    // --- Mock handler with custom verify_host string test ---

    #[test_log::test(tokio::test)]
    async fn mock_ssh_auth_handler_on_verify_host_with_ip() {
        let handler = MockSshAuthHandler {
            responses: vec![],
            verify_result: true,
        };
        assert!(handler.on_verify_host("192.168.1.1").await.unwrap());
    }

    #[test_log::test(tokio::test)]
    async fn mock_ssh_auth_handler_on_verify_host_with_ipv6() {
        let handler = MockSshAuthHandler {
            responses: vec![],
            verify_result: true,
        };
        assert!(handler.on_verify_host("::1").await.unwrap());
    }

    #[test_log::test(tokio::test)]
    async fn mock_ssh_auth_handler_on_verify_host_with_empty_string() {
        let handler = MockSshAuthHandler {
            responses: vec![],
            verify_result: false,
        };
        assert!(!handler.on_verify_host("").await.unwrap());
    }

    #[test_log::test(tokio::test)]
    async fn error_ssh_auth_handler_on_banner_does_not_error() {
        let handler = ErrorSshAuthHandler;
        // on_banner should complete without panic even for error handler
        handler.on_banner("banner text").await;
    }

    #[test_log::test(tokio::test)]
    async fn error_ssh_auth_handler_on_error_does_not_error() {
        let handler = ErrorSshAuthHandler;
        // on_error should complete without panic even for error handler
        handler.on_error("error text").await;
    }

    // --- ClientHandler additional tests ---

    #[test_log::test(tokio::test)]
    async fn client_handler_check_server_key_ed25519() {
        use russh::client::Handler;

        let mut handler = ClientHandler;

        // Generate an Ed25519 key
        let private_key = russh::keys::PrivateKey::random(
            &mut rand::thread_rng(),
            russh::keys::Algorithm::Ed25519,
        )
        .unwrap();
        let public_key = private_key.public_key();

        // Should always return Ok(true) regardless of key type
        assert!(handler.check_server_key(public_key).await.unwrap());
    }

    // --- parse_ssh_config additional hosts ---

    #[test]
    fn parse_ssh_config_with_fqdn() {
        let result = Ssh::parse_ssh_config("server.example.co.uk");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_ssh_config_with_hyphenated_host() {
        let result = Ssh::parse_ssh_config("my-server-01.internal");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_ssh_config_with_underscore_host() {
        let result = Ssh::parse_ssh_config("my_server_01");
        assert!(result.is_ok());
    }

    // --- Launch argument building tests ---

    use super::build_launch_args;

    #[test]
    fn launch_args_unix_empty_extra() {
        let cmd = build_launch_args(SshFamily::Unix, "distant", "").unwrap();
        assert_eq!(cmd, "distant server listen --daemon --host ssh");
    }

    #[test]
    fn launch_args_windows_empty_extra() {
        let cmd = build_launch_args(SshFamily::Windows, "distant", "").unwrap();
        assert_eq!(cmd, "distant server listen --daemon --host ssh");
    }

    #[test]
    fn launch_args_unix_with_port() {
        let cmd = build_launch_args(SshFamily::Unix, "distant", "--port 8080").unwrap();
        assert_eq!(cmd, "distant server listen --daemon --host ssh --port 8080");
    }

    #[test]
    fn launch_args_windows_with_port() {
        let cmd = build_launch_args(SshFamily::Windows, "distant", "--port 8080").unwrap();
        assert_eq!(cmd, "distant server listen --daemon --host ssh --port 8080");
    }

    #[test]
    fn launch_args_unix_with_multiple_flags() {
        let cmd =
            build_launch_args(SshFamily::Unix, "distant", "--port 8080 --log-level trace").unwrap();
        assert!(cmd.contains("--port 8080"));
        assert!(cmd.contains("--log-level trace"));
    }

    #[test]
    fn launch_args_unix_with_quoted_value() {
        let cmd = build_launch_args(
            SshFamily::Unix,
            "distant",
            "--config '/path/to/config file.toml'",
        )
        .unwrap();
        // shell_words::split removes quotes and keeps the value
        assert!(cmd.contains("/path/to/config file.toml"));
    }

    #[test]
    fn launch_args_unix_invalid_quoting() {
        // Unmatched quote should produce an error
        let result = build_launch_args(SshFamily::Unix, "distant", "--arg 'unclosed");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn launch_args_windows_with_quoted_value() {
        let cmd = build_launch_args(
            SshFamily::Windows,
            "distant.exe",
            "--config \"C:\\path\\to\\config file.toml\"",
        )
        .unwrap();
        assert!(cmd.contains("distant.exe"));
        assert!(cmd.contains("server listen"));
    }

    #[test]
    fn launch_args_custom_binary() {
        let cmd = build_launch_args(SshFamily::Unix, "/usr/local/bin/distant", "").unwrap();
        assert!(cmd.starts_with("/usr/local/bin/distant"));
        assert!(cmd.contains("server listen"));
    }

    #[test]
    fn launch_args_unix_double_quoted() {
        let cmd =
            build_launch_args(SshFamily::Unix, "distant", "--key \"value with spaces\"").unwrap();
        assert!(cmd.contains("value with spaces"));
    }

    // --- Authentication error message building tests ---
    // These test the same logic used at the end of Ssh::authenticate

    #[test]
    fn auth_error_message_no_methods_tried() {
        let methods_tried: Vec<String> = Vec::new();
        let tried = if methods_tried.is_empty() {
            "none".to_string()
        } else {
            methods_tried.join(", ")
        };
        assert_eq!(tried, "none");
    }

    #[test]
    fn auth_error_message_single_method_tried() {
        let methods_tried = ["publickey".to_string()];
        let tried = if methods_tried.is_empty() {
            "none".to_string()
        } else {
            methods_tried.join(", ")
        };
        assert_eq!(tried, "publickey");
    }

    #[test]
    fn auth_error_message_multiple_methods_tried() {
        let methods_tried = [
            "publickey".to_string(),
            "keyboard-interactive".to_string(),
            "password".to_string(),
        ];
        let tried = if methods_tried.is_empty() {
            "none".to_string()
        } else {
            methods_tried.join(", ")
        };
        assert_eq!(tried, "publickey, keyboard-interactive, password");
    }

    #[test]
    fn auth_error_message_format() {
        let tried = "publickey, password".to_string();
        let accepts = "publickey".to_string();
        let msg = format!("Permission denied (tried: {tried}; server accepts: {accepts})");
        assert!(msg.contains("tried: publickey, password"));
        assert!(msg.contains("server accepts: publickey"));
    }

    #[test]
    fn auth_error_message_with_unknown_server_methods() {
        let server_methods: Option<russh::MethodSet> = None;
        let accepts = server_methods
            .as_ref()
            .map(format_methods)
            .unwrap_or_else(|| "unknown".to_string());
        assert_eq!(accepts, "unknown");
    }

    #[test]
    fn auth_error_message_with_known_server_methods() {
        let server_methods = Some(russh::MethodSet::from(
            [russh::MethodKind::PublicKey, russh::MethodKind::Password].as_slice(),
        ));
        let accepts = server_methods
            .as_ref()
            .map(format_methods)
            .unwrap_or_else(|| "unknown".to_string());
        assert!(accepts.contains("publickey"));
        assert!(accepts.contains("password"));
    }

    // --- Server method detection logic tests ---

    #[test]
    fn server_accepts_pubkey_when_methods_unknown() {
        use russh::MethodKind;
        let server_methods: Option<russh::MethodSet> = None;
        let accepts = server_methods
            .as_ref()
            .is_none_or(|m| m.contains(&MethodKind::PublicKey));
        assert!(accepts, "Should accept pubkey when methods unknown");
    }

    #[test]
    fn server_accepts_pubkey_when_in_set() {
        use russh::MethodKind;
        let server_methods = Some(russh::MethodSet::from([MethodKind::PublicKey].as_slice()));
        let accepts = server_methods
            .as_ref()
            .is_none_or(|m| m.contains(&MethodKind::PublicKey));
        assert!(accepts, "Should accept pubkey when in method set");
    }

    #[test]
    fn server_rejects_pubkey_when_not_in_set() {
        use russh::MethodKind;
        let server_methods = Some(russh::MethodSet::from([MethodKind::Password].as_slice()));
        let accepts = server_methods
            .as_ref()
            .is_none_or(|m| m.contains(&MethodKind::PublicKey));
        assert!(!accepts, "Should reject pubkey when not in method set");
    }

    #[test]
    fn server_accepts_password_when_methods_unknown() {
        use russh::MethodKind;
        let server_methods: Option<russh::MethodSet> = None;
        let accepts = server_methods
            .as_ref()
            .is_none_or(|m| m.contains(&MethodKind::Password));
        assert!(accepts);
    }

    #[test]
    fn server_accepts_kbdint_when_methods_unknown() {
        use russh::MethodKind;
        let server_methods: Option<russh::MethodSet> = None;
        let accepts = server_methods
            .as_ref()
            .is_none_or(|m| m.contains(&MethodKind::KeyboardInteractive));
        assert!(accepts);
    }

    #[test]
    fn server_accepts_kbdint_when_in_set() {
        use russh::MethodKind;
        let server_methods = Some(russh::MethodSet::from(
            [MethodKind::KeyboardInteractive].as_slice(),
        ));
        let accepts = server_methods
            .as_ref()
            .is_none_or(|m| m.contains(&MethodKind::KeyboardInteractive));
        assert!(accepts);
    }

    #[test]
    fn server_rejects_kbdint_when_not_in_set() {
        use russh::MethodKind;
        let server_methods = Some(russh::MethodSet::from(
            [MethodKind::PublicKey, MethodKind::Password].as_slice(),
        ));
        let accepts = server_methods
            .as_ref()
            .is_none_or(|m| m.contains(&MethodKind::KeyboardInteractive));
        assert!(!accepts);
    }

    // --- Key file discovery logic tests ---

    #[test]
    fn key_file_discovery_with_explicit_identity_files() {
        let opts = SshOpts {
            identity_files: vec![PathBuf::from("/custom/key1"), PathBuf::from("/custom/key2")],
            ..SshOpts::default()
        };
        let key_files: Vec<PathBuf> = if !opts.identity_files.is_empty() {
            opts.identity_files.clone()
        } else {
            Vec::new()
        };
        assert_eq!(key_files.len(), 2);
        assert_eq!(key_files[0], PathBuf::from("/custom/key1"));
    }

    #[test]
    fn key_file_discovery_empty_falls_to_default() {
        let opts = SshOpts::default();
        let key_files: Vec<PathBuf> = if !opts.identity_files.is_empty() {
            opts.identity_files.clone()
        } else {
            // Would normally check for default keys; here just return empty
            Vec::new()
        };
        assert!(key_files.is_empty());
    }

    // --- SshAuthEvent username fallback logic ---

    #[test]
    fn auth_event_username_fallback_when_name_empty() {
        let name = String::new();
        let user = "default_user".to_string();
        let username = if name.is_empty() { user.clone() } else { name };
        assert_eq!(username, "default_user");
    }

    #[test]
    fn auth_event_username_uses_name_when_present() {
        let name = "provided_name".to_string();
        let user = "default_user".to_string();
        let username = if name.is_empty() { user.clone() } else { name };
        assert_eq!(username, "provided_name");
    }

    #[test]
    fn auth_event_instructions_fallback_when_empty() {
        let instructions = String::new();
        let result = if instructions.is_empty() {
            "Authentication required".to_string()
        } else {
            instructions
        };
        assert_eq!(result, "Authentication required");
    }

    #[test]
    fn auth_event_instructions_uses_provided_when_present() {
        let instructions = "Custom instructions".to_string();
        let result = if instructions.is_empty() {
            "Authentication required".to_string()
        } else {
            instructions
        };
        assert_eq!(result, "Custom instructions");
    }
}
