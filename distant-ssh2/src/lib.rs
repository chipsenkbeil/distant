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
use std::io::{self, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use distant_core::net::auth::{DummyAuthHandler, Verifier};
use distant_core::net::client::{Client, ClientConfig};
use distant_core::net::common::{InmemoryTransport, OneshotListener};
use distant_core::net::server::{Server, ServerRef};
use distant_core::{DistantApiServerHandler, DistantClient, DistantSingleKeyCredentials};
use log::*;
use russh::client::{self, Handle};
use russh::keys::PrivateKey;
use ssh2_config_rs::{HostParams, ParseRule, SshConfig};
use tokio::sync::Mutex;

mod api;
mod process;
mod utils;

use api::SshDistantApi;

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
pub struct DistantLaunchOpts {
    /// Binary to use for distant server
    pub binary: String,

    /// Arguments to supply to the distant server when starting it
    pub args: String,

    /// Timeout to use when connecting to the distant server
    pub timeout: Duration,
}

impl Default for DistantLaunchOpts {
    fn default() -> Self {
        Self {
            binary: String::from("distant"),
            args: String::new(),
            timeout: Duration::from_secs(15),
        }
    }
}

/// Interface to handle various events during ssh authentication
#[async_trait]
pub trait SshAuthHandler {
    /// Invoked whenever a series of authentication prompts need to be displayed and responded to,
    /// receiving one event at a time and returning a collection of answers matching the total
    /// prompts provided in the event
    async fn on_authenticate(&self, event: SshAuthEvent) -> io::Result<Vec<String>>;

    /// Invoked when the host is unknown for a new ssh connection, receiving the host as a str and
    /// returning true if the host is acceptable or false if the host (and thereby ssh client)
    /// should be declined
    async fn on_verify_host(&self, host: &str) -> io::Result<bool>;

    /// Invoked when receiving a banner from the ssh server, receiving the banner as a str, useful
    /// to display to the user
    async fn on_banner(&self, text: &str);

    /// Invoked when an error is encountered, receiving the error as a str
    async fn on_error(&self, text: &str);
}

/// Implementation of [`SshAuthHandler`] that prompts locally for authentication and verification
/// events
pub struct LocalSshAuthHandler;

#[async_trait]
impl SshAuthHandler for LocalSshAuthHandler {
    async fn on_authenticate(&self, event: SshAuthEvent) -> io::Result<Vec<String>> {
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

    async fn on_verify_host(&self, host: &str) -> io::Result<bool> {
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

    async fn on_banner(&self, _text: &str) {
        trace!("[local] on_banner({_text})");
    }

    async fn on_error(&self, _text: &str) {
        trace!("[local] on_error({_text})");
    }
}

/// russh client handler
struct ClientHandler;

#[async_trait]
impl client::Handler for ClientHandler {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::PublicKey,
    ) -> impl std::future::Future<Output = Result<bool, Self::Error>> + Send {
        // TODO: Implement proper host key verification
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

        // Basic connection attempt logging (always shown at info level)
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

        // VERBOSE MODE: Comprehensive diagnostics
        if opts.verbose {
            info!("=== SSH Verbose Mode Enabled ===");
            info!("  Target: {}:{}", host.as_ref(), port);
            info!("  User: {}", user);
            debug!("  Identity files: {:?}", opts.identity_files);
            debug!("  Identities only: {:?}", opts.identities_only);
            debug!("  Proxy command: {:?}", opts.proxy_command);
            debug!("  Known hosts files: {:?}", opts.user_known_hosts_files);
            debug!("  Russh keepalive: {:?}", config.keepalive_interval);
            info!("================================");

            // TCP CONNECTIVITY PRE-TEST (verbose mode only)
            info!(
                "Running TCP connectivity pre-test to {}:{}...",
                host.as_ref(),
                port
            );
            match tokio::time::timeout(
                Duration::from_secs(10),
                tokio::net::TcpStream::connect((host.as_ref(), port)),
            )
            .await
            {
                Ok(Ok(stream)) => {
                    let peer = stream.peer_addr()?;
                    let local = stream.local_addr()?;
                    info!("✓ TCP pre-test SUCCESS");
                    info!("  Local address: {}", local);
                    info!("  Peer address: {}", peer);
                    drop(stream); // Close test connection
                }
                Ok(Err(e)) => {
                    error!("✗ TCP pre-test FAILED");
                    error!("  Error: {}", e);
                    error!("  Error kind: {:?}", e.kind());
                    error!("  OS error code: {:?}", e.raw_os_error());
                    return Err(io::Error::new(
                        io::ErrorKind::ConnectionRefused,
                        format!(
                            "TCP connectivity pre-test failed to {}:{}: {} (kind: {:?}, os: {:?})",
                            host.as_ref(),
                            port,
                            e,
                            e.kind(),
                            e.raw_os_error()
                        ),
                    ));
                }
                Err(_) => {
                    error!("✗ TCP pre-test TIMEOUT after 10 seconds");
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!(
                            "TCP connectivity pre-test timed out to {}:{} after 10s",
                            host.as_ref(),
                            port
                        ),
                    ));
                }
            }
        }

        // RUSSH CONNECTION ATTEMPT with enhanced error handling
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
                info!("✓ SSH connection established to {}:{}", host.as_ref(), port);
                h
            }
            Err(e) => {
                // Enhanced error reporting
                error!("✗ SSH connection FAILED to {}:{}", host.as_ref(), port);
                error!("  Russh error: {}", e);
                debug!("  Russh error debug: {:?}", e);

                // Try to extract underlying IO error for detailed diagnostics
                let detailed_msg =
                    if let Some(io_err) = e.source().and_then(|s| s.downcast_ref::<io::Error>()) {
                        error!("  Underlying IO error: {}", io_err);
                        error!("  IO error kind: {:?}", io_err.kind());
                        error!("  OS error code: {:?}", io_err.raw_os_error());

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
            .map(|h| h.join(".ssh/config"))
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "No home directory found"))?;

        if !config_path.exists() {
            // Return empty host params with default algorithms
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

        // Apply SSH config algorithms if present
        config.preferred = Self::build_preferred_algorithms(params);

        // Set keepalive if configured
        if let Some(interval) = params.server_alive_interval {
            config.keepalive_interval = Some(interval);
        }

        Ok(config)
    }

    fn build_preferred_algorithms(_params: &HostParams) -> russh::Preferred {
        // TODO: Map KEX, ciphers, and MACs from SSH config to russh types
        // For now, use defaults

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
        // If already authenticated, exit
        if self.authenticated {
            return Ok(());
        }

        // Try public key authentication first
        if !self.opts.identity_files.is_empty() {
            for key_file in &self.opts.identity_files {
                match self.load_private_key(key_file).await {
                    Ok(key) => {
                        let key_with_hash = russh::keys::PrivateKeyWithHashAlg::new(
                            Arc::new(key),
                            None, // Use default hash algorithm
                        );

                        let auth_res = self
                            .handle
                            .authenticate_publickey(&self.user, key_with_hash)
                            .await
                            .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;

                        if auth_res.success() {
                            self.authenticated = true;
                            return Ok(());
                        }
                    }
                    Err(e) => {
                        warn!("Failed to load key {:?}: {}", key_file, e);
                    }
                }
            }
        }

        // Fall back to password authentication
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
            let auth_res = self
                .handle
                .authenticate_password(&self.user, password)
                .await
                .map_err(|e| io::Error::new(io::ErrorKind::PermissionDenied, e))?;

            if auth_res.success() {
                self.authenticated = true;
                Ok(())
            } else {
                Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Authentication failed",
                ))
            }
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "No password provided",
            ))
        }
    }

    async fn load_private_key(&self, path: &Path) -> io::Result<PrivateKey> {
        let contents = tokio::fs::read_to_string(path).await?;
        russh::keys::decode_secret_key(&contents, None)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Detects whether the family is Unix or Windows
    pub async fn detect_family(&self) -> io::Result<SshFamily> {
        // Check if we already have a cached value
        {
            let guard = self.cached_family.lock().await;
            if let Some(family) = *guard {
                return Ok(family);
            }
        }

        // Use utils::is_windows to detect
        let is_windows = utils::is_windows(&self.handle).await?;
        let family = if is_windows {
            SshFamily::Windows
        } else {
            SshFamily::Unix
        };

        // Cache the result
        {
            let mut guard = self.cached_family.lock().await;
            *guard = Some(family);
        }

        Ok(family)
    }

    /// Converts into a distant client
    pub async fn into_distant_client(self) -> io::Result<DistantClient> {
        let family = self.detect_family().await?;
        let api = SshDistantApi::new(self.handle, family).await?;

        // Create inmemory transport for local-to-remote API communication
        let (t1, t2) = InmemoryTransport::pair(100);

        // Start local server using our API implementation
        let server = Server::new()
            .handler(DistantApiServerHandler::new(api))
            .verifier(Verifier::none());

        tokio::spawn(async move {
            let _ = server.start(OneshotListener::from_value(t2));
        });

        // Connect to local server
        let client = Client::build()
            .auth_handler(DummyAuthHandler)
            .config(ClientConfig::default())
            .connector(t1)
            .connect()
            .await
            .map_err(io::Error::other)?;

        Ok(client)
    }

    /// Converts into a pair of distant client and server ref
    pub async fn into_distant_pair(self) -> io::Result<(DistantClient, ServerRef)> {
        let family = self.detect_family().await?;
        let api = SshDistantApi::new(self.handle, family).await?;

        // Create inmemory transport
        let (t1, t2) = InmemoryTransport::pair(100);

        // Start server
        let server = Server::new()
            .handler(DistantApiServerHandler::new(api))
            .verifier(Verifier::none());

        let server_ref = server
            .start(OneshotListener::from_value(t2))
            .map_err(io::Error::other)?;

        // Connect to server
        let client = Client::build()
            .auth_handler(DummyAuthHandler)
            .config(ClientConfig::default())
            .connector(t1)
            .connect()
            .await
            .map_err(io::Error::other)?;

        Ok((client, server_ref))
    }

    /// Launch distant server on remote machine and connect to it
    pub async fn launch(self, opts: DistantLaunchOpts) -> io::Result<DistantSingleKeyCredentials> {
        debug!("Launching distant server: {} {}", opts.binary, opts.args);

        // TODO: Implement launch logic
        // This needs to execute the distant binary on remote machine and capture output

        Err(io::Error::other(
            "Launch not yet implemented in russh migration",
        ))
    }

    /// Launch and connect to distant server
    pub async fn launch_and_connect(self, opts: DistantLaunchOpts) -> io::Result<DistantClient> {
        let _credentials = self.launch(opts).await?;

        // TODO: Parse credentials and connect

        Err(io::Error::other(
            "Launch and connect not yet implemented in russh migration",
        ))
    }
}
