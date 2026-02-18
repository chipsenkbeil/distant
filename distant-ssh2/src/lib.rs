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
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use distant_core::net::auth::{AuthHandlerMap, DummyAuthHandler, Verifier};
use distant_core::net::client::{Client, ClientConfig};
use distant_core::net::common::{InmemoryTransport, OneshotListener, Version};
use distant_core::net::server::{Server, ServerRef};
use distant_core::protocol::PROTOCOL_VERSION;
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

/// Handles SSH client events from the russh connection, including host key verification.
struct ClientHandler;

#[async_trait]
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
        if self.authenticated {
            return Ok(());
        }

        // Try public key authentication first
        if !self.opts.identity_files.is_empty() {
            for key_file in &self.opts.identity_files {
                match self.load_private_key(key_file).await {
                    Ok(key) => {
                        let key_with_hash =
                            russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), None);

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
    pub async fn into_distant_client(self) -> io::Result<DistantClient> {
        let family = self.detect_family().await?;
        let api = SshDistantApi::new(self.handle, family);

        let (t1, t2) = InmemoryTransport::pair(100);

        let server = Server::new()
            .handler(DistantApiServerHandler::new(api))
            .verifier(Verifier::none());

        tokio::spawn(async move {
            let _ = server.start(OneshotListener::from_value(t2));
        });

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
        let api = SshDistantApi::new(self.handle, family);

        let (t1, t2) = InmemoryTransport::pair(100);

        let server = Server::new()
            .handler(DistantApiServerHandler::new(api))
            .verifier(Verifier::none());

        let server_ref = server
            .start(OneshotListener::from_value(t2))
            .map_err(io::Error::other)?;

        let client = Client::build()
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
    pub async fn launch(self, opts: DistantLaunchOpts) -> io::Result<DistantSingleKeyCredentials> {
        debug!("Launching distant server: {} {}", opts.binary, opts.args);

        let family = self.detect_family().await?;
        trace!("Detected family: {}", family.as_static_str());

        use distant_core::net::common::Host;

        let host = self
            .host()
            .parse::<Host>()
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidInput, x))?;

        // Open a channel and request a PTY for launching the distant server
        let channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(io::Error::other)?;

        channel
            .request_pty(true, "xterm-256color", 80, 24, 0, 0, &[])
            .await
            .map_err(io::Error::other)?;

        channel
            .request_shell(true)
            .await
            .map_err(io::Error::other)?;

        // Build arguments for distant to run listen subcommand
        let mut args = vec![
            String::from("server"),
            String::from("listen"),
            String::from("--daemon"),
            String::from("--host"),
            String::from("ssh"),
        ];
        args.extend(match family {
            SshFamily::Windows => winsplit::split(&opts.args),
            SshFamily::Unix => shell_words::split(&opts.args)
                .map_err(|x| io::Error::new(io::ErrorKind::InvalidInput, x))?,
        });

        let cmd = format!("{} {}\r\n", opts.binary, args.join(" "));
        debug!("Writing launch command: {}", cmd.trim());

        use std::io::Cursor;
        channel
            .data(Cursor::new(cmd.into_bytes()))
            .await
            .map_err(io::Error::other)?;

        // Read stdout from the PTY and look for credentials
        let (mut read_half, _write_half) = channel.split();

        let timeout = opts.timeout;
        let start_instant = std::time::Instant::now();
        let mut stdout = Vec::new();

        loop {
            // Check for timeout
            if start_instant.elapsed() >= timeout {
                // Clean the bytes before including by removing anything that isn't ascii
                // and isn't a control character (except whitespace)
                stdout.retain(|b: &u8| {
                    b.is_ascii() && (b.is_ascii_whitespace() || !b.is_ascii_control())
                });

                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!(
                        "Failed to spawn server: '{}'",
                        shell_words::quote(&String::from_utf8_lossy(&stdout))
                    ),
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
                        DistantSingleKeyCredentials::find_lax(&String::from_utf8_lossy(&stdout))
                    {
                        credentials.host = host;
                        debug!("Got credentials from launched server");
                        return Ok(credentials);
                    }
                }
                Ok(Some(_)) => {
                    // Other channel messages, continue
                }
                Ok(None) => {
                    // Channel closed without finding credentials
                    stdout.retain(|b: &u8| {
                        b.is_ascii() && (b.is_ascii_whitespace() || !b.is_ascii_control())
                    });
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        format!(
                            "Channel closed before credentials found: '{}'",
                            shell_words::quote(&String::from_utf8_lossy(&stdout))
                        ),
                    ));
                }
                Err(_) => {
                    // Timeout on this read iteration, will be caught at loop top
                }
            }
        }
    }

    /// Consume [`Ssh`] and launch a distant server, then connect to it as a client.
    pub async fn launch_and_connect(self, opts: DistantLaunchOpts) -> io::Result<DistantClient> {
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
            match Client::tcp(addr)
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
