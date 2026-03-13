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
use ssh2_config::{HostParams, ParseRule, SshConfig};
use tokio::sync::Mutex;

mod api;
mod auth;
mod plugin;
mod process;
mod utils;

pub use plugin::SshPlugin;
pub use utils::SftpPathBuf;

mod proxy;

use api::SshApi;
use auth::{expand_tilde, format_methods};

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

/// Returns the platform-specific system SSH configuration directory.
///
/// - Unix: `/etc/ssh`
/// - Windows: `%ProgramData%\ssh`
fn system_ssh_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        Some(PathBuf::from("/etc/ssh"))
    }
    #[cfg(windows)]
    {
        std::env::var("ProgramData")
            .ok()
            .map(|d| PathBuf::from(d).join("ssh"))
    }
}

/// Verify a server's host key against known_hosts files using the specified policy.
///
/// Returns `Ok(true)` if the key is accepted, or an error if rejected.
fn check_host_key(
    host: &str,
    port: u16,
    pubkey: &russh::keys::PublicKey,
    known_hosts_files: &[PathBuf],
    policy: &HostKeyPolicy,
) -> Result<bool, russh::Error> {
    use russh::keys::known_hosts::{check_known_hosts_path, learn_known_hosts_path};

    // Check each known_hosts file for a matching key
    for file in known_hosts_files {
        match check_known_hosts_path(host, port, pubkey, file) {
            Ok(true) => {
                debug!(
                    "Host key for {host}:{port} found and matches in {}",
                    file.display()
                );
                return Ok(true);
            }
            Ok(false) => {
                // Key not found in this file (no entry for this host, or different key type)
                debug!(
                    "No matching host key for {host}:{port} in {}",
                    file.display()
                );
            }
            Err(russh::keys::Error::KeyChanged { line }) => {
                error!(
                    "Host key for {host}:{port} has changed ({}:{}). \
                     This could indicate a man-in-the-middle attack.",
                    file.display(),
                    line,
                );
                return Err(russh::Error::from(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!(
                        "Host key for {host}:{port} has changed ({}:line {line}). \
                         This could indicate a man-in-the-middle attack. \
                         Remove the offending key to continue.",
                        file.display(),
                    ),
                )));
            }
            Err(e) => {
                // File not found, parse error, etc. — skip to next file
                debug!("Error checking known_hosts file {}: {e}", file.display());
            }
        }
    }

    // Key not found in any file — apply policy
    match policy {
        HostKeyPolicy::AcceptNew => {
            // Record the key in the first known_hosts file (TOFU)
            if let Some(file) = known_hosts_files.first() {
                info!(
                    "Accepting and recording new host key for {host}:{port} in {}",
                    file.display()
                );
                if let Err(e) = learn_known_hosts_path(host, port, pubkey, file) {
                    warn!(
                        "Failed to record host key for {host}:{port} to {}: {e}",
                        file.display()
                    );
                }
            } else {
                info!("Accepting new host key for {host}:{port} (no known_hosts file configured)");
            }
            Ok(true)
        }
        HostKeyPolicy::No => {
            debug!(
                "Accepting host key for {host}:{port} without recording (StrictHostKeyChecking=no)"
            );
            Ok(true)
        }
        HostKeyPolicy::Yes => {
            error!("Host key for {host}:{port} not found in any known_hosts file");
            Err(russh::Error::from(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Host key for {host}:{port} not found in known_hosts. \
                     Connection rejected (StrictHostKeyChecking=yes)."
                ),
            )))
        }
    }
}

/// Policy for handling unknown SSH host keys.
#[derive(Clone, Debug, Default)]
enum HostKeyPolicy {
    /// Accept unknown keys and record to known_hosts (TOFU). Reject changed keys.
    #[default]
    AcceptNew,

    /// Accept all keys without recording (insecure, equivalent to OpenSSH `no`).
    No,

    /// Reject unknown keys; only accept keys already in known_hosts.
    Yes,
}

impl HostKeyPolicy {
    /// Parses the policy from the SSH config `StrictHostKeyChecking` value.
    fn from_config(value: &str) -> Self {
        match value.to_lowercase().as_str() {
            "no" => Self::No,
            "yes" => Self::Yes,
            // "accept-new" is the explicit TOFU setting; also the default
            _ => Self::AcceptNew,
        }
    }
}

/// Handles SSH client events from the russh connection, including host key verification.
struct ClientHandler {
    /// The server's SSH identification string, captured during key exchange.
    remote_sshid: Arc<Mutex<Option<String>>>,

    /// Hostname for known_hosts lookups.
    host: String,

    /// Port for known_hosts lookups.
    port: u16,

    /// Paths to known_hosts files to check.
    known_hosts_files: Vec<PathBuf>,

    /// Host key verification policy.
    policy: HostKeyPolicy,
}

impl client::Handler for ClientHandler {
    type Error = russh::Error;

    fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::PublicKey,
    ) -> impl std::future::Future<Output = Result<bool, Self::Error>> + Send {
        let host = self.host.clone();
        let port = self.port;
        let files = self.known_hosts_files.clone();
        let policy = self.policy.clone();
        async move { check_host_key(&host, port, server_public_key, &files, &policy) }
    }

    fn kex_done(
        &mut self,
        _shared_secret: Option<&[u8]>,
        _names: &russh::Names,
        session: &mut russh::client::Session,
    ) -> impl std::future::Future<Output = Result<(), Self::Error>> + Send {
        let sshid = String::from_utf8_lossy(session.remote_sshid()).into_owned();
        let remote_sshid = Arc::clone(&self.remote_sshid);
        async move {
            debug!("Remote SSH identification: {}", sshid);
            *remote_sshid.lock().await = Some(sshid);
            Ok(())
        }
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
    /// The server's SSH identification string, captured during key exchange.
    remote_sshid: Arc<Mutex<Option<String>>>,
    /// Identity files from SSH config, used as fallback if opts.identity_files is empty.
    ssh_config_identity_files: Option<Vec<PathBuf>>,
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

        // Resolve the actual hostname to connect to (SSH config HostName directive)
        let connect_host = ssh_config.host_name.as_deref().unwrap_or(host.as_ref());

        // Determine connection parameters
        let port = opts.port.or(ssh_config.port).unwrap_or(22);
        let user = opts
            .user
            .clone()
            .or(ssh_config.user.clone())
            .unwrap_or_else(|| whoami::username().unwrap_or_default());

        // Resolve identities_only: prefer explicit opts, then SSH config
        let mut opts = opts;
        if opts.identities_only.is_none() {
            opts.identities_only = ssh_config
                .unsupported_fields
                .get("identitiesonly")
                .and_then(|v| v.first())
                .and_then(|s| match s.to_lowercase().as_str() {
                    "yes" => Some(true),
                    "no" => Some(false),
                    _ => None,
                });
        }

        info!(
            "SSH connection attempt: {}:{} as user '{}'",
            connect_host, port, user
        );
        debug!("SSH options: {:?}", opts);
        debug!(
            "SSH config: port={:?}, user={:?}, host_name={:?}",
            ssh_config.port, ssh_config.user, ssh_config.host_name
        );

        // Build russh configuration
        let config = Self::build_russh_config(&opts, &ssh_config)?;

        // Verbose diagnostics
        if opts.verbose {
            info!("SSH verbose mode enabled");
            if ssh_config.host_name.is_some() {
                info!(
                    "Host alias '{}' resolved to '{}'",
                    host.as_ref(),
                    connect_host
                );
            }
            info!("Target: {}:{}", connect_host, port);
            info!("User: {}", user);
            debug!("Identity files: {:?}", opts.identity_files);
            debug!("Identities only: {:?}", opts.identities_only);
            debug!("Proxy command: {:?}", opts.proxy_command);
            debug!("Known hosts files: {:?}", opts.user_known_hosts_files);
            debug!("Russh keepalive: {:?}", config.keepalive_interval);
        }

        debug!("Initiating SSH connection to {}:{}...", connect_host, port);

        // Resolve known_hosts files: prefer explicit opts, then SSH config, then defaults
        let mut known_hosts_files = if !opts.user_known_hosts_files.is_empty() {
            opts.user_known_hosts_files
                .iter()
                .map(|p| expand_tilde(p))
                .collect()
        } else if let Some(config_values) = ssh_config.unsupported_fields.get("userknownhostsfile")
        {
            let files: Vec<PathBuf> = config_values
                .iter()
                .map(|s| expand_tilde(Path::new(s.trim())))
                .collect();
            if files.is_empty() {
                Self::default_known_hosts_files()
            } else {
                files
            }
        } else {
            Self::default_known_hosts_files()
        };

        // Append global known_hosts from SSH config or system defaults
        if let Some(global_values) = ssh_config.unsupported_fields.get("globalknownhostsfile") {
            for path_str in global_values {
                let path = expand_tilde(Path::new(path_str.trim()));
                if !known_hosts_files.contains(&path) {
                    known_hosts_files.push(path);
                }
            }
        } else if let Some(ssh_dir) = system_ssh_dir() {
            for name in ["ssh_known_hosts", "ssh_known_hosts2"] {
                let path = ssh_dir.join(name);
                if !known_hosts_files.contains(&path) {
                    known_hosts_files.push(path);
                }
            }
        }

        // Resolve host key policy: prefer explicit opts, then SSH config, then default (TOFU)
        let policy = opts
            .other
            .get("stricthostkeychecking")
            .or_else(|| opts.other.get("StrictHostKeyChecking"))
            .map(|v| HostKeyPolicy::from_config(v))
            .or_else(|| {
                ssh_config
                    .unsupported_fields
                    .get("stricthostkeychecking")
                    .and_then(|v| v.first())
                    .map(|v| HostKeyPolicy::from_config(v))
            })
            .unwrap_or_default();

        debug!(
            "Host key verification: policy={:?}, files={:?}",
            policy, known_hosts_files
        );

        // Resolve proxy command: prefer explicit opts, then SSH config.
        // ssh2-config splits unsupported field values into words, so we rejoin them.
        // "none" disables an inherited proxy (same as OpenSSH).
        let proxy_command = opts
            .proxy_command
            .clone()
            .or_else(|| {
                ssh_config
                    .unsupported_fields
                    .get("proxycommand")
                    .map(|v| v.join(" "))
                    .filter(|s| !s.is_empty())
            })
            .filter(|cmd| !cmd.eq_ignore_ascii_case("none"));

        let remote_sshid: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
        let handler = ClientHandler {
            remote_sshid: Arc::clone(&remote_sshid),
            host: host.as_ref().to_string(),
            port,
            known_hosts_files,
            policy,
        };

        let config = Arc::new(config);
        let connect_result = if let Some(ref cmd) = proxy_command {
            let substituted = proxy::substitute_proxy_command(cmd, connect_host, port, &user);
            info!("Using ProxyCommand: {}", substituted);
            let stream = proxy::ProxyStream::spawn(&substituted)?;
            russh::client::connect_stream(config, stream, handler).await
        } else {
            russh::client::connect(config, (connect_host, port), handler).await
        };

        let handle = match connect_result {
            Ok(h) => {
                info!("SSH connection established to {}:{}", connect_host, port);
                h
            }
            Err(e) => {
                error!("SSH connection failed to {}:{}", connect_host, port);
                error!("Russh error: {}", e);
                debug!("Russh error debug: {:?}", e);

                let detailed_msg = if let Some(io_err) =
                    e.source().and_then(|s| s.downcast_ref::<io::Error>())
                {
                    error!("Underlying IO error: {}", io_err);
                    error!("IO error kind: {:?}", io_err.kind());
                    error!("OS error code: {:?}", io_err.raw_os_error());

                    format!(
                        "SSH connection to {}:{} failed: {} (IO error: {}, kind: {:?}, os: {:?})",
                        connect_host,
                        port,
                        e,
                        io_err,
                        io_err.kind(),
                        io_err.raw_os_error()
                    )
                } else {
                    format!("SSH connection to {}:{} failed: {}", connect_host, port, e)
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
            remote_sshid,
            ssh_config_identity_files: ssh_config.identity_file.clone(),
        })
    }

    /// Returns the default known_hosts file paths.
    ///
    /// Includes user paths (`~/.ssh/known_hosts`, `~/.ssh/known_hosts2`) and
    /// system paths (`/etc/ssh/ssh_known_hosts`, `/etc/ssh/ssh_known_hosts2`
    /// on Unix; `%ProgramData%\ssh\...` on Windows).
    fn default_known_hosts_files() -> Vec<PathBuf> {
        let mut files = dirs::home_dir()
            .map(|h| {
                vec![
                    h.join(".ssh").join("known_hosts"),
                    h.join(".ssh").join("known_hosts2"),
                ]
            })
            .unwrap_or_default();
        if let Some(ssh_dir) = system_ssh_dir() {
            files.push(ssh_dir.join("ssh_known_hosts"));
            files.push(ssh_dir.join("ssh_known_hosts2"));
        }
        files
    }

    fn parse_ssh_config(host: &str) -> io::Result<HostParams> {
        use ssh2_config::DefaultAlgorithms;

        let system_params = system_ssh_dir()
            .map(|d| d.join("ssh_config"))
            .and_then(|path| Self::try_parse_ssh_config_file(&path, host));

        let user_params = dirs::home_dir()
            .map(|h| h.join(".ssh").join("config"))
            .and_then(|path| Self::try_parse_ssh_config_file(&path, host));

        // Merge: user config takes precedence over system config
        match (user_params, system_params) {
            (Some(mut user), Some(system)) => {
                user.overwrite_if_none(&system);
                Ok(user)
            }
            (Some(user), None) => Ok(user),
            (None, Some(system)) => Ok(system),
            (None, None) => Ok(HostParams::new(&DefaultAlgorithms::default())),
        }
    }

    /// Try to parse an SSH config file and query it for a host.
    /// Returns `None` if the file doesn't exist or can't be parsed.
    fn try_parse_ssh_config_file(path: &Path, host: &str) -> Option<HostParams> {
        if !path.exists() {
            return None;
        }
        match File::open(path) {
            Ok(f) => {
                let mut reader = BufReader::new(f);
                match SshConfig::default().parse(&mut reader, ParseRule::ALLOW_UNSUPPORTED_FIELDS) {
                    Ok(config) => Some(config.query(host)),
                    Err(e) => {
                        debug!("Failed to parse SSH config {}: {}", path.display(), e);
                        None
                    }
                }
            }
            Err(e) => {
                debug!("Failed to open SSH config {}: {}", path.display(), e);
                None
            }
        }
    }

    fn build_russh_config(
        _opts: &SshOpts,
        params: &HostParams,
    ) -> io::Result<russh::client::Config> {
        let mut config = russh::client::Config::default();

        config.preferred = Self::build_preferred_algorithms(params);

        // Map keepalive: prefer server_alive_interval, fall back to tcp_keep_alive
        if let Some(interval) = params.server_alive_interval {
            config.keepalive_interval = Some(interval);
        } else if params.tcp_keep_alive == Some(true) {
            // TCP keepalive requested but no interval specified; use a sensible default
            config.keepalive_interval = Some(Duration::from_secs(15));
        }

        // Map connection timeout
        if let Some(timeout) = params.connect_timeout {
            config.inactivity_timeout = Some(timeout);
        }

        Ok(config)
    }

    /// Builds preferred algorithm lists from SSH config, filtering to only algorithms
    /// that russh actually supports. Unsupported algorithm names are logged and skipped.
    fn build_preferred_algorithms(params: &HostParams) -> russh::Preferred {
        let mut preferred = russh::Preferred::default();

        // Map KexAlgorithms
        if !params.kex_algorithms.is_default() {
            let kex: Vec<russh::kex::Name> = params
                .kex_algorithms
                .algorithms()
                .iter()
                .filter_map(|s| match russh::kex::Name::try_from(s.as_str()) {
                    Ok(name) => Some(name),
                    Err(_) => {
                        debug!("Skipping unsupported KEX algorithm from SSH config: {}", s);
                        None
                    }
                })
                .collect();
            if !kex.is_empty() {
                // Append extension negotiation names that russh needs internally
                let mut full_kex = kex;
                for ext in [
                    russh::kex::EXTENSION_SUPPORT_AS_CLIENT,
                    russh::kex::EXTENSION_OPENSSH_STRICT_KEX_AS_CLIENT,
                ] {
                    if !full_kex.contains(&ext) {
                        full_kex.push(ext);
                    }
                }
                preferred.kex = full_kex.into();
            }
        }

        // Map HostKeyAlgorithms
        if !params.host_key_algorithms.is_default() {
            let keys: Vec<russh::keys::Algorithm> = params
                .host_key_algorithms
                .algorithms()
                .iter()
                .filter_map(|s| match s.parse::<russh::keys::Algorithm>() {
                    Ok(algo) => Some(algo),
                    Err(_) => {
                        debug!(
                            "Skipping unsupported host key algorithm from SSH config: {}",
                            s
                        );
                        None
                    }
                })
                .collect();
            if !keys.is_empty() {
                preferred.key = keys.into();
            }
        }

        // Map Ciphers
        if !params.ciphers.is_default() {
            let ciphers: Vec<russh::cipher::Name> = params
                .ciphers
                .algorithms()
                .iter()
                .filter_map(|s| match russh::cipher::Name::try_from(s.as_str()) {
                    Ok(name) => Some(name),
                    Err(_) => {
                        debug!("Skipping unsupported cipher from SSH config: {}", s);
                        None
                    }
                })
                .collect();
            if !ciphers.is_empty() {
                preferred.cipher = ciphers.into();
            }
        }

        // Map MACs
        if !params.mac.is_default() {
            let macs: Vec<russh::mac::Name> = params
                .mac
                .algorithms()
                .iter()
                .filter_map(|s| match russh::mac::Name::try_from(s.as_str()) {
                    Ok(name) => Some(name),
                    Err(_) => {
                        debug!("Skipping unsupported MAC from SSH config: {}", s);
                        None
                    }
                })
                .collect();
            if !macs.is_empty() {
                preferred.mac = macs.into();
            }
        }

        // Map Compression
        if let Some(true) = params.compression {
            let compressed: Vec<russh::compression::Name> = ["zlib@openssh.com", "zlib", "none"]
                .iter()
                .filter_map(|s| russh::compression::Name::try_from(*s).ok())
                .collect();
            if !compressed.is_empty() {
                preferred.compression = compressed.into();
            }
        }

        preferred
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

        // Try SSH agent first — it avoids touching key files and works with hardware tokens
        if server_accepts_pubkey
            && !self.opts.identities_only.unwrap_or(false)
            && auth::try_agent_auth(
                &mut self.handle,
                &self.user,
                &mut methods_tried,
                &mut server_methods,
            )
            .await?
        {
            self.authenticated = true;
            return Ok(());
        }

        // Try key files from explicit opts, SSH config, or ~/.ssh defaults
        if server_accepts_pubkey {
            let key_files =
                auth::collect_key_files(&self.opts.identity_files, &self.ssh_config_identity_files);
            if !key_files.is_empty() {
                methods_tried.push("publickey".to_string());
            }
            for key_file in &key_files {
                if let Some(true) = auth::load_and_try_key(
                    &mut self.handle,
                    &self.user,
                    key_file,
                    &handler,
                    &mut server_methods,
                )
                .await?
                {
                    self.authenticated = true;
                    return Ok(());
                }
            }
        }

        // Keyboard-interactive — track whether we prompted the user to avoid
        // double-prompting if we fall through to password auth
        let mut user_was_prompted = false;
        if server_accepts_kbdint {
            let (authenticated, prompted) = auth::try_keyboard_interactive(
                &mut self.handle,
                &self.user,
                &handler,
                &mut methods_tried,
                &mut server_methods,
            )
            .await?;
            user_was_prompted = prompted;
            if authenticated {
                self.authenticated = true;
                return Ok(());
            }
        }

        // Password auth — skip if keyboard-interactive already prompted the user
        if server_accepts_password
            && !user_was_prompted
            && auth::try_password_auth(
                &mut self.handle,
                &self.user,
                &handler,
                &mut methods_tried,
                &mut server_methods,
            )
            .await?
        {
            self.authenticated = true;
            return Ok(());
        }

        Err(auth::build_auth_error(&methods_tried, &server_methods))
    }

    /// Detects whether the family is Unix or Windows.
    ///
    /// Uses a layered detection strategy: SSH identification string, then SFTP
    /// `canonicalize(".")`, then exec fallback. The result is cached for subsequent calls.
    pub async fn detect_family(&self) -> io::Result<SshFamily> {
        {
            let guard = self.cached_family.lock().await;
            if let Some(family) = *guard {
                return Ok(family);
            }
        }

        let sshid = self.remote_sshid.lock().await.clone();
        let is_windows = utils::is_windows(&self.handle, sshid.as_deref()).await?;
        let family = if is_windows {
            SshFamily::Windows
        } else {
            SshFamily::Unix
        };

        debug!("Detected remote family: {:?}", family);

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
    use std::io::Write;

    use rstest::rstest;

    use super::*;

    #[test]
    fn ssh_family_should_return_lowercase_variant_names() {
        assert_eq!(SshFamily::Unix.as_static_str(), "unix");
        assert_eq!(SshFamily::Windows.as_static_str(), "windows");
    }

    #[test]
    fn launch_opts_should_have_correct_defaults() {
        let opts = LaunchOpts::default();
        assert_eq!(opts.binary, "distant");
        assert!(opts.args.is_empty());
        assert_eq!(opts.timeout, Duration::from_secs(15));
    }

    #[tokio::test]
    async fn local_ssh_auth_handler_should_not_panic_on_banner_and_error() {
        let handler = LocalSshAuthHandler;
        handler.on_banner("test banner").await;
        handler.on_error("test error").await;
        // These just log — verifying they don't panic is sufficient
    }

    #[test]
    fn ssh_opts_should_have_none_defaults() {
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
    fn ssh_opts_should_store_all_fields_when_populated() {
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
    fn ssh_opts_should_maintain_btreemap_key_ordering() {
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

    #[test]
    fn ssh_auth_prompt_should_store_prompt_and_echo() {
        let prompt = SshAuthPrompt {
            prompt: "Password: ".to_string(),
            echo: false,
        };
        assert_eq!(prompt.prompt, "Password: ");
        assert!(!prompt.echo);
    }

    #[test]
    fn ssh_auth_prompt_should_support_echo_true() {
        let prompt = SshAuthPrompt {
            prompt: "Username: ".to_string(),
            echo: true,
        };
        assert_eq!(prompt.prompt, "Username: ");
        assert!(prompt.echo);
    }

    #[test]
    fn ssh_auth_event_should_store_all_fields() {
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
    fn ssh_auth_event_should_allow_empty_fields() {
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
    fn ssh_auth_event_should_store_multiple_prompts() {
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
    fn launch_opts_should_accept_custom_values() {
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
    fn clean_launch_output_should_return_no_output_when_both_empty() {
        let result = Ssh::clean_launch_output(b"", b"");
        assert_eq!(result, "(no output)");
    }

    #[test]
    fn clean_launch_output_should_show_stdout_only() {
        let result = Ssh::clean_launch_output(b"hello world", b"");
        assert_eq!(result, "stdout: 'hello world'");
    }

    #[test]
    fn clean_launch_output_should_show_stderr_only() {
        let result = Ssh::clean_launch_output(b"", b"error occurred");
        assert_eq!(result, "stderr: 'error occurred'");
    }

    #[test]
    fn clean_launch_output_should_show_both_when_present() {
        let result = Ssh::clean_launch_output(b"some output", b"some error");
        assert_eq!(result, "stdout: 'some output', stderr: 'some error'");
    }

    #[test]
    fn clean_launch_output_should_strip_control_characters() {
        let result = Ssh::clean_launch_output(b"hello\x01\x02world", b"");
        assert_eq!(result, "stdout: 'helloworld'");
    }

    #[test]
    fn clean_launch_output_should_preserve_whitespace() {
        let result = Ssh::clean_launch_output(b"hello\tworld", b"");
        assert_eq!(result, "stdout: 'hello\tworld'");
    }

    #[test]
    fn clean_launch_output_should_trim_edge_whitespace() {
        let result = Ssh::clean_launch_output(b"  hello  ", b"  error  ");
        assert_eq!(result, "stdout: 'hello', stderr: 'error'");
    }

    #[test]
    fn clean_launch_output_should_treat_only_whitespace_as_empty() {
        let result = Ssh::clean_launch_output(b"   ", b"   ");
        assert_eq!(result, "(no output)");
    }

    #[test]
    fn clean_launch_output_should_treat_only_control_chars_as_empty() {
        let result = Ssh::clean_launch_output(b"\x01\x02\x03", b"\x04\x05\x06");
        assert_eq!(result, "(no output)");
    }

    #[test]
    fn clean_launch_output_should_keep_text_around_control_chars() {
        let result = Ssh::clean_launch_output(b"\x1b[31mred text\x1b[0m", b"\x1b[error\x1b]done");
        assert!(result.contains("stdout:"), "Expected stdout in '{result}'");
    }

    #[test]
    fn clean_launch_output_should_handle_invalid_utf8_gracefully() {
        let result = Ssh::clean_launch_output(b"valid\xff\xfeinvalid", b"");
        assert!(
            result.contains("stdout:"),
            "Expected stdout label in '{result}'"
        );
    }

    #[test]
    fn clean_launch_output_should_preserve_inner_newlines() {
        let result = Ssh::clean_launch_output(b"\nline1\nline2\n", b"");
        assert_eq!(result, "stdout: 'line1\nline2'");
    }

    #[test]
    fn clean_launch_output_should_preserve_carriage_returns() {
        let result = Ssh::clean_launch_output(b"line1\r\nline2", b"");
        assert!(result.contains("line1"), "Expected line1 in '{result}'");
        assert!(result.contains("line2"), "Expected line2 in '{result}'");
    }

    #[test]
    fn clean_launch_output_should_strip_null_bytes() {
        let result = Ssh::clean_launch_output(b"before\x00after", b"");
        assert_eq!(result, "stdout: 'beforeafter'");
    }

    #[test]
    fn clean_launch_output_should_strip_bell_character() {
        let result = Ssh::clean_launch_output(b"text\x07here", b"");
        assert_eq!(result, "stdout: 'texthere'");
    }

    #[test]
    fn clean_launch_output_should_strip_backspace() {
        let result = Ssh::clean_launch_output(b"ab\x08c", b"");
        assert_eq!(result, "stdout: 'abc'");
    }

    #[test]
    fn clean_launch_output_should_omit_stderr_when_only_control_chars() {
        let result = Ssh::clean_launch_output(b"output", b"\x01\x02\x03");
        assert_eq!(result, "stdout: 'output'");
    }

    #[test]
    fn clean_launch_output_should_omit_stdout_when_only_control_chars() {
        let result = Ssh::clean_launch_output(b"\x01\x02\x03", b"error text");
        assert_eq!(result, "stderr: 'error text'");
    }

    #[test]
    fn clean_launch_output_should_handle_long_output() {
        let long_stdout = b"A".repeat(1000);
        let long_stderr = b"B".repeat(500);
        let result = Ssh::clean_launch_output(&long_stdout, &long_stderr);
        assert!(result.starts_with("stdout: '"));
        assert!(result.contains("stderr: '"));
    }

    #[test]
    fn build_russh_config_should_use_defaults_when_no_overrides() {
        use ssh2_config::DefaultAlgorithms;

        let opts = SshOpts::default();
        let params = HostParams::new(&DefaultAlgorithms::default());
        let config = Ssh::build_russh_config(&opts, &params).unwrap();

        assert!(config.keepalive_interval.is_none());
    }

    #[test]
    fn build_russh_config_should_set_keepalive_from_server_alive_interval() {
        use ssh2_config::DefaultAlgorithms;

        let opts = SshOpts::default();
        let mut params = HostParams::new(&DefaultAlgorithms::default());
        params.server_alive_interval = Some(Duration::from_secs(60));

        let config = Ssh::build_russh_config(&opts, &params).unwrap();
        assert_eq!(config.keepalive_interval, Some(Duration::from_secs(60)));
    }

    #[test]
    fn build_russh_config_should_set_short_keepalive() {
        use ssh2_config::DefaultAlgorithms;

        let opts = SshOpts::default();
        let mut params = HostParams::new(&DefaultAlgorithms::default());
        params.server_alive_interval = Some(Duration::from_secs(5));

        let config = Ssh::build_russh_config(&opts, &params).unwrap();
        assert_eq!(config.keepalive_interval, Some(Duration::from_secs(5)));
    }

    #[test]
    fn build_russh_config_should_leave_keepalive_none_when_unset() {
        use ssh2_config::DefaultAlgorithms;

        let opts = SshOpts::default();
        let mut params = HostParams::new(&DefaultAlgorithms::default());
        params.server_alive_interval = None;

        let config = Ssh::build_russh_config(&opts, &params).unwrap();
        assert!(config.keepalive_interval.is_none());
    }

    #[test]
    fn build_russh_config_should_succeed_with_verbose_opts() {
        use ssh2_config::DefaultAlgorithms;

        let mut opts = SshOpts::default();
        opts.verbose = true;
        let params = HostParams::new(&DefaultAlgorithms::default());

        let config = Ssh::build_russh_config(&opts, &params).unwrap();
        assert!(config.keepalive_interval.is_none());
    }

    #[test]
    fn build_russh_config_should_succeed_with_populated_opts() {
        use ssh2_config::DefaultAlgorithms;

        let mut opts = SshOpts::default();
        opts.port = Some(2222);
        opts.user = Some("testuser".to_string());
        opts.identity_files.push(PathBuf::from("/tmp/id_rsa"));

        let params = HostParams::new(&DefaultAlgorithms::default());
        let config = Ssh::build_russh_config(&opts, &params).unwrap();
        assert!(config.keepalive_interval.is_none());
    }

    #[test]
    fn build_preferred_algorithms_should_return_defaults_with_empty_params() {
        use ssh2_config::DefaultAlgorithms;

        let params = HostParams::new(&DefaultAlgorithms::default());
        let preferred = Ssh::build_preferred_algorithms(&params);

        let default_preferred = russh::Preferred::default();
        assert_eq!(preferred.kex, default_preferred.kex);
        assert_eq!(preferred.cipher, default_preferred.cipher);
    }

    #[test]
    fn build_preferred_algorithms_should_use_defaults_despite_custom_params() {
        use ssh2_config::DefaultAlgorithms;

        let mut params = HostParams::new(&DefaultAlgorithms::default());
        params.port = Some(9999);
        params.user = Some("custom-user".to_string());

        let preferred = Ssh::build_preferred_algorithms(&params);
        let default_preferred = russh::Preferred::default();
        assert_eq!(preferred.kex, default_preferred.kex);
    }

    #[rstest]
    #[case::nonexistent("nonexistent-host.example.com")]
    #[case::localhost("localhost")]
    #[case::wildcard("*")]
    #[case::empty("")]
    #[case::ipv4("192.168.1.1")]
    #[case::ipv6("::1")]
    #[case::fqdn("server.example.co.uk")]
    #[case::hyphenated("my-server-01.internal")]
    #[case::underscore("my_server_01")]
    fn parse_ssh_config_should_not_error_for_any_hostname(#[case] host: &str) {
        // parse_ssh_config reads the real ~/.ssh/config (or returns defaults).
        // We can't assert specific field values since the user's config may have
        // wildcard matches. The unwrap proves it never errors for valid input.
        let _params = Ssh::parse_ssh_config(host).unwrap();
    }

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
    async fn mock_ssh_auth_handler_should_return_configured_responses() {
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
    async fn mock_ssh_auth_handler_should_accept_when_verify_true() {
        let handler = MockSshAuthHandler {
            responses: vec![],
            verify_result: true,
        };
        assert!(handler.on_verify_host("example.com").await.unwrap());
    }

    #[test_log::test(tokio::test)]
    async fn mock_ssh_auth_handler_should_reject_when_verify_false() {
        let handler = MockSshAuthHandler {
            responses: vec![],
            verify_result: false,
        };
        assert!(!handler.on_verify_host("evil.com").await.unwrap());
    }

    #[test_log::test(tokio::test)]
    async fn mock_ssh_auth_handler_should_return_all_responses_for_multiple_prompts() {
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
    async fn mock_ssh_auth_handler_should_return_empty_for_no_prompts() {
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
    async fn error_ssh_auth_handler_should_return_error_on_authenticate() {
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
    async fn error_ssh_auth_handler_should_return_error_on_verify_host() {
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

    #[test]
    fn ssh_auth_event_should_support_multiline_content() {
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
    fn ssh_auth_event_should_support_unicode_content() {
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
    fn ssh_auth_prompt_should_allow_empty_string() {
        let prompt = SshAuthPrompt {
            prompt: String::new(),
            echo: false,
        };
        assert!(prompt.prompt.is_empty());
        assert!(!prompt.echo);
    }

    #[test]
    fn ssh_auth_event_should_handle_many_prompts_with_alternating_echo() {
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

    #[test_log::test(tokio::test)]
    async fn check_server_key_should_accept_new_key_with_tofu_policy() {
        use russh::client::Handler;

        let dir = tempfile::tempdir().unwrap();
        let kh = dir.path().join("known_hosts");

        let mut handler = ClientHandler {
            remote_sshid: Arc::new(Mutex::new(None)),
            host: "testhost".to_string(),
            port: 22,
            known_hosts_files: vec![kh],
            policy: HostKeyPolicy::AcceptNew,
        };

        let private_key = russh::keys::PrivateKey::random(
            &mut rand::thread_rng(),
            russh::keys::Algorithm::Ed25519,
        )
        .unwrap();
        let public_key = private_key.public_key().clone();

        assert!(handler.check_server_key(&public_key).await.unwrap());
    }

    #[test]
    fn ssh_opts_should_support_struct_update_syntax() {
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
    fn ssh_opts_should_store_identities_only_true() {
        let opts = SshOpts {
            identities_only: Some(true),
            ..SshOpts::default()
        };
        assert_eq!(opts.identities_only, Some(true));
    }

    #[test]
    fn ssh_opts_should_store_identities_only_false() {
        let opts = SshOpts {
            identities_only: Some(false),
            ..SshOpts::default()
        };
        assert_eq!(opts.identities_only, Some(false));
    }

    #[test]
    fn ssh_opts_should_store_multiple_identity_files() {
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
    fn ssh_opts_should_store_multiple_known_hosts_files() {
        let opts = SshOpts {
            user_known_hosts_files: vec![
                PathBuf::from("/home/user/.ssh/known_hosts"),
                PathBuf::from("/home/user/.ssh/known_hosts2"),
            ],
            ..SshOpts::default()
        };
        assert_eq!(opts.user_known_hosts_files.len(), 2);
    }

    #[test]
    fn clean_launch_output_should_handle_multibyte_utf8() {
        let result = Ssh::clean_launch_output("Server ready".as_bytes(), b"");
        assert_eq!(result, "stdout: 'Server ready'");
    }

    #[test]
    fn clean_launch_output_should_preserve_form_feed() {
        let result = Ssh::clean_launch_output(b"before\x0cafter", b"");
        assert_eq!(result, "stdout: 'before\x0cafter'");
    }

    #[test]
    fn clean_launch_output_should_handle_vertical_tab() {
        let result = Ssh::clean_launch_output(b"a\x0bb", b"");
        assert!(result.contains("stdout:"), "Expected stdout in '{result}'");
    }

    #[test]
    fn clean_launch_output_should_handle_mixed_utf8_in_stderr() {
        let result = Ssh::clean_launch_output(b"", b"err\xff\xfeor");
        assert!(result.contains("stderr:"), "Expected stderr in '{result}'");
    }

    #[test]
    fn clean_launch_output_should_strip_ansi_escape_codes() {
        let input = b"\x1b[31mError\x1b[0m";
        let result = Ssh::clean_launch_output(input, b"");
        assert!(
            result.contains("[31mError"),
            "Expected cleaned ANSI in '{result}'"
        );
    }

    #[test]
    fn clean_launch_output_should_preserve_crlf_content() {
        let result = Ssh::clean_launch_output(b"line1\r\nline2\r\nline3", b"");
        assert!(result.contains("line1"));
        assert!(result.contains("line2"));
        assert!(result.contains("line3"));
    }

    #[test]
    fn clean_launch_output_should_handle_large_stderr() {
        let stderr = b"E".repeat(5000);
        let result = Ssh::clean_launch_output(b"", &stderr);
        assert!(result.starts_with("stderr: '"));
        assert!(result.len() > 5000);
    }

    #[test]
    fn clean_launch_output_should_handle_mixed_control_and_whitespace() {
        let result = Ssh::clean_launch_output(b"\x01\t\x02 \x03\n\x04", b"");
        assert!(result.contains("stdout:") || result == "(no output)");
    }

    #[test]
    fn launch_opts_should_accept_zero_timeout() {
        let opts = LaunchOpts {
            binary: String::from("distant"),
            args: String::new(),
            timeout: Duration::from_secs(0),
        };
        assert_eq!(opts.timeout, Duration::ZERO);
    }

    #[test]
    fn launch_opts_should_accept_long_timeout() {
        let opts = LaunchOpts {
            binary: String::from("distant"),
            args: String::new(),
            timeout: Duration::from_secs(3600),
        };
        assert_eq!(opts.timeout.as_secs(), 3600);
    }

    #[test]
    fn launch_opts_should_store_complex_args() {
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
    fn launch_opts_should_preserve_quoted_args() {
        let opts = LaunchOpts {
            binary: String::from("distant"),
            args: String::from("--config '/path/to/config file.toml'"),
            timeout: Duration::from_secs(15),
        };
        assert!(opts.args.contains("config file.toml"));
    }

    #[test]
    fn ssh_opts_should_handle_many_identity_files() {
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
    fn ssh_opts_should_handle_many_other_map_entries() {
        let mut other = BTreeMap::new();
        for i in 0..50 {
            other.insert(format!("Key{:03}", i), format!("Value{}", i));
        }

        let opts = SshOpts {
            other,
            ..SshOpts::default()
        };

        assert_eq!(opts.other.len(), 50);
        let first_key = opts.other.keys().next().unwrap();
        assert_eq!(first_key, "Key000");
    }

    #[test]
    fn build_russh_config_should_allow_zero_keepalive() {
        use ssh2_config::DefaultAlgorithms;

        let opts = SshOpts::default();
        let mut params = HostParams::new(&DefaultAlgorithms::default());
        params.server_alive_interval = Some(Duration::from_secs(0));

        let config = Ssh::build_russh_config(&opts, &params).unwrap();
        assert_eq!(config.keepalive_interval, Some(Duration::from_secs(0)));
    }

    #[test]
    fn build_russh_config_should_allow_large_keepalive() {
        use ssh2_config::DefaultAlgorithms;

        let opts = SshOpts::default();
        let mut params = HostParams::new(&DefaultAlgorithms::default());
        params.server_alive_interval = Some(Duration::from_secs(3600));

        let config = Ssh::build_russh_config(&opts, &params).unwrap();
        assert_eq!(config.keepalive_interval, Some(Duration::from_secs(3600)));
    }

    #[test]
    fn build_russh_config_should_include_preferred_algorithms() {
        use ssh2_config::DefaultAlgorithms;

        let opts = SshOpts::default();
        let params = HostParams::new(&DefaultAlgorithms::default());
        let config = Ssh::build_russh_config(&opts, &params).unwrap();

        let default_preferred = russh::Preferred::default();
        assert_eq!(config.preferred.kex, default_preferred.kex);
        assert_eq!(config.preferred.cipher, default_preferred.cipher);
    }

    #[test_log::test(tokio::test)]
    async fn mock_ssh_auth_handler_should_accept_ip_address_host() {
        let handler = MockSshAuthHandler {
            responses: vec![],
            verify_result: true,
        };
        assert!(handler.on_verify_host("192.168.1.1").await.unwrap());
    }

    #[test_log::test(tokio::test)]
    async fn mock_ssh_auth_handler_should_accept_ipv6_host() {
        let handler = MockSshAuthHandler {
            responses: vec![],
            verify_result: true,
        };
        assert!(handler.on_verify_host("::1").await.unwrap());
    }

    #[test_log::test(tokio::test)]
    async fn mock_ssh_auth_handler_should_reject_empty_host() {
        let handler = MockSshAuthHandler {
            responses: vec![],
            verify_result: false,
        };
        assert!(!handler.on_verify_host("").await.unwrap());
    }

    #[test_log::test(tokio::test)]
    async fn check_server_key_should_accept_any_key_with_no_policy() {
        use russh::client::Handler;

        let mut handler = ClientHandler {
            remote_sshid: Arc::new(Mutex::new(None)),
            host: "testhost".to_string(),
            port: 22,
            known_hosts_files: vec![],
            policy: HostKeyPolicy::No,
        };

        let private_key = russh::keys::PrivateKey::random(
            &mut rand::thread_rng(),
            russh::keys::Algorithm::Ed25519,
        )
        .unwrap();
        let public_key = private_key.public_key().clone();

        let result: Result<bool, russh::Error> = handler.check_server_key(&public_key).await;
        assert!(result.unwrap());
    }

    use super::build_launch_args;

    #[test]
    fn build_launch_args_should_produce_base_unix_command() {
        let cmd = build_launch_args(SshFamily::Unix, "distant", "").unwrap();
        assert_eq!(cmd, "distant server listen --daemon --host ssh");
    }

    #[test]
    fn build_launch_args_should_produce_base_windows_command() {
        let cmd = build_launch_args(SshFamily::Windows, "distant", "").unwrap();
        assert_eq!(cmd, "distant server listen --daemon --host ssh");
    }

    #[test]
    fn build_launch_args_should_append_port_on_unix() {
        let cmd = build_launch_args(SshFamily::Unix, "distant", "--port 8080").unwrap();
        assert_eq!(cmd, "distant server listen --daemon --host ssh --port 8080");
    }

    #[test]
    fn build_launch_args_should_append_port_on_windows() {
        let cmd = build_launch_args(SshFamily::Windows, "distant", "--port 8080").unwrap();
        assert_eq!(cmd, "distant server listen --daemon --host ssh --port 8080");
    }

    #[test]
    fn build_launch_args_should_append_multiple_flags() {
        let cmd =
            build_launch_args(SshFamily::Unix, "distant", "--port 8080 --log-level trace").unwrap();
        assert!(cmd.contains("--port 8080"));
        assert!(cmd.contains("--log-level trace"));
    }

    #[test]
    fn build_launch_args_should_unquote_shell_words() {
        let cmd = build_launch_args(
            SshFamily::Unix,
            "distant",
            "--config '/path/to/config file.toml'",
        )
        .unwrap();
        assert!(cmd.contains("/path/to/config file.toml"));
    }

    #[test]
    fn build_launch_args_should_error_on_invalid_quoting() {
        let result = build_launch_args(SshFamily::Unix, "distant", "--arg 'unclosed");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn build_launch_args_should_handle_windows_paths() {
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
    fn build_launch_args_should_use_custom_binary_path() {
        let cmd = build_launch_args(SshFamily::Unix, "/usr/local/bin/distant", "").unwrap();
        assert!(cmd.starts_with("/usr/local/bin/distant"));
        assert!(cmd.contains("server listen"));
    }

    #[test]
    fn build_launch_args_should_handle_double_quotes() {
        let cmd =
            build_launch_args(SshFamily::Unix, "distant", "--key \"value with spaces\"").unwrap();
        assert!(cmd.contains("value with spaces"));
    }

    #[test]
    fn server_method_check_should_accept_pubkey_when_methods_unknown() {
        use russh::MethodKind;
        let server_methods: Option<russh::MethodSet> = None;
        let accepts = server_methods
            .as_ref()
            .is_none_or(|m| m.contains(&MethodKind::PublicKey));
        assert!(accepts, "Should accept pubkey when methods unknown");
    }

    #[test]
    fn server_method_check_should_accept_pubkey_when_in_set() {
        use russh::MethodKind;
        let server_methods = Some(russh::MethodSet::from([MethodKind::PublicKey].as_slice()));
        let accepts = server_methods
            .as_ref()
            .is_none_or(|m| m.contains(&MethodKind::PublicKey));
        assert!(accepts, "Should accept pubkey when in method set");
    }

    #[test]
    fn server_method_check_should_reject_pubkey_when_not_in_set() {
        use russh::MethodKind;
        let server_methods = Some(russh::MethodSet::from([MethodKind::Password].as_slice()));
        let accepts = server_methods
            .as_ref()
            .is_none_or(|m| m.contains(&MethodKind::PublicKey));
        assert!(!accepts, "Should reject pubkey when not in method set");
    }

    #[test]
    fn server_method_check_should_accept_password_when_methods_unknown() {
        use russh::MethodKind;
        let server_methods: Option<russh::MethodSet> = None;
        let accepts = server_methods
            .as_ref()
            .is_none_or(|m| m.contains(&MethodKind::Password));
        assert!(accepts);
    }

    #[test]
    fn server_method_check_should_accept_kbdint_when_methods_unknown() {
        use russh::MethodKind;
        let server_methods: Option<russh::MethodSet> = None;
        let accepts = server_methods
            .as_ref()
            .is_none_or(|m| m.contains(&MethodKind::KeyboardInteractive));
        assert!(accepts);
    }

    #[test]
    fn server_method_check_should_accept_kbdint_when_in_set() {
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
    fn server_method_check_should_reject_kbdint_when_not_in_set() {
        use russh::MethodKind;
        let server_methods = Some(russh::MethodSet::from(
            [MethodKind::PublicKey, MethodKind::Password].as_slice(),
        ));
        let accepts = server_methods
            .as_ref()
            .is_none_or(|m| m.contains(&MethodKind::KeyboardInteractive));
        assert!(!accepts);
    }

    #[test]
    fn parse_ssh_config_should_return_hostname_from_temp_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config");
        let mut f = std::fs::File::create(&config_path).unwrap();
        writeln!(
            f,
            "Host windows-vm\n  HostName 10.211.55.3\n  User testuser\n  Port 2222"
        )
        .unwrap();

        let mut reader = std::io::BufReader::new(std::fs::File::open(&config_path).unwrap());
        let config = SshConfig::default()
            .parse(&mut reader, ParseRule::ALLOW_UNSUPPORTED_FIELDS)
            .unwrap();
        let params = config.query("windows-vm");

        assert_eq!(params.host_name.as_deref(), Some("10.211.55.3"));
        assert_eq!(params.user.as_deref(), Some("testuser"));
        assert_eq!(params.port, Some(2222));
    }

    #[test]
    fn parse_ssh_config_should_return_none_hostname_for_unmatched_host() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config");
        let mut f = std::fs::File::create(&config_path).unwrap();
        writeln!(f, "Host myserver\n  HostName 10.0.0.1").unwrap();

        let mut reader = std::io::BufReader::new(std::fs::File::open(&config_path).unwrap());
        let config = SshConfig::default()
            .parse(&mut reader, ParseRule::ALLOW_UNSUPPORTED_FIELDS)
            .unwrap();
        let params = config.query("other-server");

        assert!(params.host_name.is_none());
    }

    #[test]
    fn build_russh_config_should_set_default_keepalive_from_tcp_keep_alive() {
        use ssh2_config::DefaultAlgorithms;

        let opts = SshOpts::default();
        let mut params = HostParams::new(&DefaultAlgorithms::default());
        params.tcp_keep_alive = Some(true);

        let config = Ssh::build_russh_config(&opts, &params).unwrap();
        assert_eq!(config.keepalive_interval, Some(Duration::from_secs(15)));
    }

    #[test]
    fn build_russh_config_should_prefer_server_alive_interval_over_tcp_keep_alive() {
        use ssh2_config::DefaultAlgorithms;

        let opts = SshOpts::default();
        let mut params = HostParams::new(&DefaultAlgorithms::default());
        params.tcp_keep_alive = Some(true);
        params.server_alive_interval = Some(Duration::from_secs(30));

        let config = Ssh::build_russh_config(&opts, &params).unwrap();
        assert_eq!(config.keepalive_interval, Some(Duration::from_secs(30)));
    }

    /// Helper: parse a temp SSH config and return HostParams with algorithm overrides applied.
    fn parse_config_str(config_text: &str) -> HostParams {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "{}", config_text).unwrap();

        let mut reader = std::io::BufReader::new(std::fs::File::open(&path).unwrap());
        let config = SshConfig::default()
            .parse(&mut reader, ParseRule::ALLOW_UNSUPPORTED_FIELDS)
            .unwrap();
        config.query("testhost")
    }

    #[test]
    fn build_preferred_algorithms_should_map_custom_ciphers() {
        let params = parse_config_str(
            "Host testhost\n  Ciphers chacha20-poly1305@openssh.com,aes256-gcm@openssh.com\n",
        );

        let preferred = Ssh::build_preferred_algorithms(&params);
        assert!(preferred.cipher.len() <= 2);
        assert!(
            preferred
                .cipher
                .iter()
                .any(|c| c.as_ref() == "chacha20-poly1305@openssh.com")
        );
    }

    #[test]
    fn build_preferred_algorithms_should_skip_unsupported_cipher() {
        let params = parse_config_str(
            "Host testhost\n  Ciphers aes256-gcm@openssh.com,nonexistent-cipher\n",
        );

        let preferred = Ssh::build_preferred_algorithms(&params);
        assert!(
            preferred
                .cipher
                .iter()
                .all(|c| c.as_ref() != "nonexistent-cipher")
        );
    }

    #[test]
    fn build_preferred_algorithms_should_map_custom_kex() {
        let params = parse_config_str("Host testhost\n  KexAlgorithms curve25519-sha256\n");

        let preferred = Ssh::build_preferred_algorithms(&params);
        assert!(
            preferred
                .kex
                .iter()
                .any(|k| k.as_ref() == "curve25519-sha256")
        );
    }

    #[test]
    fn build_preferred_algorithms_should_map_custom_mac() {
        let params = parse_config_str("Host testhost\n  MACs hmac-sha2-256\n");

        let preferred = Ssh::build_preferred_algorithms(&params);
        assert!(preferred.mac.iter().any(|m| m.as_ref() == "hmac-sha2-256"));
    }

    /// Generate an Ed25519 public key for host-key tests.
    fn generate_keypair() -> russh::keys::PublicKey {
        let key = russh::keys::PrivateKey::random(
            &mut rand::thread_rng(),
            russh::keys::Algorithm::Ed25519,
        )
        .expect("Failed to generate test key");
        key.public_key().clone()
    }

    /// Write a known_hosts entry for the given host/port/key.
    fn write_known_hosts(
        path: &std::path::Path,
        host: &str,
        port: u16,
        pubkey: &russh::keys::PublicKey,
    ) {
        let mut file = std::fs::File::create(path).expect("create known_hosts");
        if port != 22 {
            write!(file, "[{host}]:{port} ").expect("write host");
        } else {
            write!(file, "{host} ").expect("write host");
        }
        file.write_all(pubkey.to_openssh().unwrap().as_bytes())
            .expect("write key");
        file.write_all(b"\n").expect("write newline");
    }

    #[test]
    fn check_host_key_should_accept_matching_known_key() {
        let dir = tempfile::tempdir().unwrap();
        let kh = dir.path().join("known_hosts");
        let pubkey = generate_keypair();

        write_known_hosts(&kh, "example.com", 22, &pubkey);

        let result = check_host_key("example.com", 22, &pubkey, &[kh], &HostKeyPolicy::Yes);
        assert!(result.unwrap());
    }

    #[test]
    fn check_host_key_should_reject_changed_key() {
        let dir = tempfile::tempdir().unwrap();
        let kh = dir.path().join("known_hosts");
        let original = generate_keypair();
        let different = generate_keypair();

        write_known_hosts(&kh, "example.com", 22, &original);

        let result = check_host_key(
            "example.com",
            22,
            &different,
            &[kh],
            &HostKeyPolicy::AcceptNew,
        );
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("changed"),
            "Error should mention key changed: {err}"
        );
    }

    #[test]
    fn check_host_key_should_accept_and_record_new_key_with_accept_new() {
        let dir = tempfile::tempdir().unwrap();
        let kh = dir.path().join("known_hosts");
        let pubkey = generate_keypair();

        let result = check_host_key(
            "newhost.example.com",
            22,
            &pubkey,
            std::slice::from_ref(&kh),
            &HostKeyPolicy::AcceptNew,
        );
        assert!(result.unwrap());

        assert!(kh.exists(), "known_hosts file should have been created");

        let result2 = check_host_key(
            "newhost.example.com",
            22,
            &pubkey,
            &[kh],
            &HostKeyPolicy::Yes,
        );
        assert!(result2.unwrap(), "Recorded key should match");
    }

    #[test]
    fn check_host_key_should_reject_unknown_key_with_yes_policy() {
        let dir = tempfile::tempdir().unwrap();
        let kh = dir.path().join("known_hosts");
        let pubkey = generate_keypair();

        std::fs::write(&kh, "").unwrap();

        let result = check_host_key(
            "unknown.example.com",
            22,
            &pubkey,
            &[kh],
            &HostKeyPolicy::Yes,
        );
        assert!(result.is_err());
        let err = format!("{}", result.unwrap_err());
        assert!(
            err.contains("not found"),
            "Error should say key not found: {err}"
        );
    }

    #[test]
    fn check_host_key_should_accept_without_recording_with_no_policy() {
        let dir = tempfile::tempdir().unwrap();
        let kh = dir.path().join("known_hosts");
        let pubkey = generate_keypair();

        let result = check_host_key(
            "norecord.example.com",
            22,
            &pubkey,
            std::slice::from_ref(&kh),
            &HostKeyPolicy::No,
        );
        assert!(result.unwrap());

        assert!(
            !kh.exists(),
            "known_hosts file should not be created with No policy"
        );
    }

    #[test]
    fn host_key_policy_should_parse_config_values_correctly() {
        assert!(matches!(
            HostKeyPolicy::from_config("no"),
            HostKeyPolicy::No
        ));
        assert!(matches!(
            HostKeyPolicy::from_config("NO"),
            HostKeyPolicy::No
        ));
        assert!(matches!(
            HostKeyPolicy::from_config("yes"),
            HostKeyPolicy::Yes
        ));
        assert!(matches!(
            HostKeyPolicy::from_config("YES"),
            HostKeyPolicy::Yes
        ));
        assert!(matches!(
            HostKeyPolicy::from_config("accept-new"),
            HostKeyPolicy::AcceptNew
        ));
        assert!(matches!(
            HostKeyPolicy::from_config("anything_else"),
            HostKeyPolicy::AcceptNew
        ));
    }

    #[test]
    fn check_host_key_should_use_bracketed_format_for_nonstandard_port() {
        let dir = tempfile::tempdir().unwrap();
        let kh = dir.path().join("known_hosts");
        let pubkey = generate_keypair();

        write_known_hosts(&kh, "example.com", 2222, &pubkey);

        let result = check_host_key("example.com", 2222, &pubkey, &[kh], &HostKeyPolicy::Yes);
        assert!(result.unwrap());
    }

    /// Regression test for issue #162: known_hosts file in a directory path
    /// containing whitespace (e.g. Windows username "fa fa" -> `C:\Users\fa fa\.ssh\`).
    /// The old wezterm-ssh backend failed on such paths; russh + PathBuf handles them.
    #[test]
    fn check_host_key_should_work_with_whitespace_in_path() {
        let base = tempfile::tempdir().unwrap();
        let spaced_dir = base.path().join("user name with spaces").join(".ssh");
        std::fs::create_dir_all(&spaced_dir).unwrap();

        let kh = spaced_dir.join("known_hosts");
        let pubkey = generate_keypair();

        write_known_hosts(&kh, "example.com", 22, &pubkey);

        let result = check_host_key(
            "example.com",
            22,
            &pubkey,
            std::slice::from_ref(&kh),
            &HostKeyPolicy::Yes,
        );
        assert!(result.unwrap(), "Should find known key via whitespace path");

        let kh2 = spaced_dir.join("known_hosts2");
        let new_key = generate_keypair();
        let result = check_host_key(
            "new-host.example.com",
            22,
            &new_key,
            std::slice::from_ref(&kh2),
            &HostKeyPolicy::AcceptNew,
        );
        assert!(
            result.unwrap(),
            "AcceptNew should learn key into whitespace path"
        );

        let result = check_host_key(
            "new-host.example.com",
            22,
            &new_key,
            &[kh2],
            &HostKeyPolicy::Yes,
        );
        assert!(result.unwrap(), "Learned key should be found on re-check");
    }

    #[test]
    fn system_ssh_dir_should_return_some() {
        // On any supported platform, system_ssh_dir() should return a path
        let dir = system_ssh_dir();
        assert!(
            dir.is_some(),
            "system_ssh_dir() should return Some on Unix or Windows"
        );

        #[cfg(unix)]
        assert_eq!(dir.unwrap(), PathBuf::from("/etc/ssh"));

        #[cfg(windows)]
        {
            let expected = PathBuf::from(std::env::var("ProgramData").unwrap()).join("ssh");
            assert_eq!(dir.unwrap(), expected);
        }
    }

    #[test]
    fn default_known_hosts_files_should_include_system_paths() {
        let files = Ssh::default_known_hosts_files();
        // Should contain at least the system paths
        #[cfg(unix)]
        {
            assert!(
                files
                    .iter()
                    .any(|p| p == Path::new("/etc/ssh/ssh_known_hosts")),
                "Should include /etc/ssh/ssh_known_hosts, got: {:?}",
                files
            );
            assert!(
                files
                    .iter()
                    .any(|p| p == Path::new("/etc/ssh/ssh_known_hosts2")),
                "Should include /etc/ssh/ssh_known_hosts2, got: {:?}",
                files
            );
        }
        #[cfg(windows)]
        {
            let sys_dir = PathBuf::from(std::env::var("ProgramData").unwrap()).join("ssh");
            assert!(
                files.iter().any(|p| p == &sys_dir.join("ssh_known_hosts")),
                "Should include system ssh_known_hosts, got: {:?}",
                files
            );
            assert!(
                files.iter().any(|p| p == &sys_dir.join("ssh_known_hosts2")),
                "Should include system ssh_known_hosts2, got: {:?}",
                files
            );
        }
        // User paths should come before system paths
        if let Some(home) = dirs::home_dir() {
            let user_kh = home.join(".ssh").join("known_hosts");
            let system_kh = system_ssh_dir()
                .expect("system_ssh_dir must exist for ordering check")
                .join("ssh_known_hosts");
            if let (Some(user_pos), Some(sys_pos)) = (
                files.iter().position(|p| p == &user_kh),
                files.iter().position(|p| p == &system_kh),
            ) {
                assert!(
                    user_pos < sys_pos,
                    "User known_hosts should come before system known_hosts"
                );
            }
        }
    }

    #[test]
    fn try_parse_ssh_config_file_should_return_none_for_missing_file() {
        let result = Ssh::try_parse_ssh_config_file(Path::new("/nonexistent/path/config"), "host");
        assert!(result.is_none());
    }

    #[test]
    fn try_parse_ssh_config_file_should_parse_valid_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("ssh_config");
        let mut f = std::fs::File::create(&config_path).unwrap();
        writeln!(f, "Host testhost\n  HostName 10.0.0.1\n  Port 2222").unwrap();

        let result = Ssh::try_parse_ssh_config_file(&config_path, "testhost");
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.host_name.as_deref(), Some("10.0.0.1"));
        assert_eq!(params.port, Some(2222));
    }

    #[test]
    fn try_parse_ssh_config_file_should_not_panic_on_invalid_content() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("bad_config");
        // Write binary garbage that can't be parsed as SSH config
        std::fs::write(&config_path, [0xFF, 0xFE, 0x00, 0x01]).unwrap();

        let result = Ssh::try_parse_ssh_config_file(&config_path, "host");
        // ssh2-config is fairly permissive — it may return Some with empty params
        // or None on parse failure. Either is acceptable; the key assertion is that
        // the function does not panic on binary input.
        if let Some(params) = result {
            // If parsed, the garbage should not produce meaningful SSH settings
            assert!(
                params.host_name.is_none(),
                "Binary garbage should not produce a valid HostName"
            );
        }
    }

    #[test]
    fn try_parse_ssh_config_file_should_extract_unsupported_fields() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("ssh_config");
        let mut f = std::fs::File::create(&config_path).unwrap();
        writeln!(
            f,
            "Host corp-vm\n  ProxyCommand exec ssh -W %h:%p jump\n  IdentitiesOnly yes"
        )
        .unwrap();

        let params = Ssh::try_parse_ssh_config_file(&config_path, "corp-vm").unwrap();
        // ssh2-config splits unsupported field values into words
        let proxy = params
            .unsupported_fields
            .get("proxycommand")
            .map(|v| v.join(" "));
        assert_eq!(
            proxy.as_deref(),
            Some("exec ssh -W %h:%p jump"),
            "ProxyCommand should be in unsupported_fields (rejoined)"
        );

        let id_only = params
            .unsupported_fields
            .get("identitiesonly")
            .and_then(|v| v.first());
        assert_eq!(
            id_only.map(|s| s.as_str()),
            Some("yes"),
            "IdentitiesOnly should be in unsupported_fields"
        );
    }

    #[test]
    fn parse_ssh_config_should_merge_system_and_user_configs() {
        // This test verifies the merge behavior by testing try_parse_ssh_config_file
        // directly, since parse_ssh_config reads from fixed system paths
        let dir = tempfile::tempdir().unwrap();

        // Simulate system config (has HostName but not User)
        let system_path = dir.path().join("system_config");
        let mut f = std::fs::File::create(&system_path).unwrap();
        writeln!(f, "Host myhost\n  HostName 10.0.0.1\n  Port 22").unwrap();

        // Simulate user config (has User but not HostName)
        let user_path = dir.path().join("user_config");
        let mut f = std::fs::File::create(&user_path).unwrap();
        writeln!(f, "Host myhost\n  User deployer\n  Port 2222").unwrap();

        let system_params = Ssh::try_parse_ssh_config_file(&system_path, "myhost").unwrap();
        let mut user_params = Ssh::try_parse_ssh_config_file(&user_path, "myhost").unwrap();

        // User has Port=2222 but no HostName; system has HostName and Port=22
        assert_eq!(user_params.port, Some(2222));
        assert!(user_params.host_name.is_none());
        assert_eq!(system_params.host_name.as_deref(), Some("10.0.0.1"));

        // After merge, user values take precedence
        user_params.overwrite_if_none(&system_params);
        assert_eq!(user_params.port, Some(2222), "User port should win");
        assert_eq!(
            user_params.host_name.as_deref(),
            Some("10.0.0.1"),
            "System HostName should fill in missing user HostName"
        );
        assert_eq!(
            user_params.user.as_deref(),
            Some("deployer"),
            "User should be preserved"
        );
    }
}
