#[cfg(not(any(feature = "libssh", feature = "ssh2")))]
compile_error!("Either feature \"libssh\" or \"ssh2\" must be enabled for this crate.");

use async_compat::CompatExt;
use async_trait::async_trait;
use distant_core::{
    net::{
        FramedTransport, IntoSplit, OneshotListener, ServerExt, ServerRef, TcpClientExt,
        XChaCha20Poly1305Codec,
    },
    BoxedDistantReader, BoxedDistantWriter, BoxedDistantWriterReader, DistantApiServer,
    DistantChannelExt, DistantClient, DistantSingleKeyCredentials,
};
use log::*;
use smol::channel::Receiver as SmolReceiver;
use std::{
    collections::BTreeMap,
    fmt,
    io::{self, Write},
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    str::FromStr,
    time::Duration,
};
use wezterm_ssh::{Config as WezConfig, Session as WezSession, SessionEvent as WezSessionEvent};

mod api;
mod process;

use api::SshDistantApi;

/// Represents the backend to use for ssh operations
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum SshBackend {
    /// Use libssh as backend
    #[cfg(feature = "libssh")]
    LibSsh,

    /// Use ssh2 as backend
    #[cfg(feature = "ssh2")]
    Ssh2,
}

impl SshBackend {
    pub fn as_static_str(&self) -> &'static str {
        match self {
            #[cfg(feature = "libssh")]
            Self::LibSsh => "libssh",

            #[cfg(feature = "ssh2")]
            Self::Ssh2 => "ssh2",
        }
    }
}

impl Default for SshBackend {
    /// Defaults to ssh2 if enabled, otherwise uses libssh by default
    ///
    /// NOTE: There are currently bugs in libssh that cause our implementation to hang related to
    ///       process stdout/stderr and maybe other logic.
    fn default() -> Self {
        #[cfg(feature = "ssh2")]
        {
            Self::Ssh2
        }

        #[cfg(not(feature = "ssh2"))]
        {
            Self::LibSsh
        }
    }
}

impl FromStr for SshBackend {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            #[cfg(feature = "ssh2")]
            s if s.trim().eq_ignore_ascii_case("ssh2") => Ok(Self::Ssh2),

            #[cfg(feature = "libssh")]
            s if s.trim().eq_ignore_ascii_case("libssh") => Ok(Self::LibSsh),

            _ => Err("SSH backend must be \"libssh\" or \"ssh2\""),
        }
    }
}

impl fmt::Display for SshBackend {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            #[cfg(feature = "libssh")]
            Self::LibSsh => write!(f, "libssh"),

            #[cfg(feature = "ssh2")]
            Self::Ssh2 => write!(f, "ssh2"),
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
    /// Represents the backend to use for ssh operations
    pub backend: SshBackend,

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

    /// If true, launches via `echo distant listen ... | $SHELL -l`, otherwise attempts to launch
    /// by directly invoking distant
    pub use_login_shell: bool,

    /// Timeout to use when connecting to the distant server
    pub timeout: Duration,
}

impl Default for DistantLaunchOpts {
    fn default() -> Self {
        Self {
            binary: String::from("distant"),
            args: String::new(),
            use_login_shell: false,
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
                    eprintln!("{}", line);
                }

                let answer = if prompt.echo {
                    eprint!("{}", prompt_line);
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

        task.await
            .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?
    }

    async fn on_verify_host(&self, host: &str) -> io::Result<bool> {
        eprintln!("{}", host);
        let task = tokio::task::spawn_blocking(|| {
            eprint!("Enter [y/N]> ");
            std::io::stderr().lock().flush()?;

            let mut answer = String::new();
            std::io::stdin().read_line(&mut answer)?;

            match answer.as_str() {
                "y" | "Y" | "yes" | "YES" => Ok(true),
                _ => Ok(false),
            }
        });

        task.await
            .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?
    }

    async fn on_banner(&self, _text: &str) {}

    async fn on_error(&self, _text: &str) {}
}

/// Represents an ssh2 client
pub struct Ssh {
    session: WezSession,
    events: SmolReceiver<WezSessionEvent>,
    host: String,
    port: u16,
    authenticated: bool,
}

impl Ssh {
    /// Connect to a remote TCP server using SSH
    pub fn connect(host: impl AsRef<str>, opts: SshOpts) -> io::Result<Self> {
        debug!(
            "Establishing ssh connection to {} using {:?}",
            host.as_ref(),
            opts
        );
        let mut config = WezConfig::new();
        config.add_default_config_files();

        // Grab the config for the specific host
        let mut config = config.for_host(host.as_ref());

        // Override config with any settings provided by client opts
        if let Some(port) = opts.port.as_ref() {
            config.insert("port".to_string(), port.to_string());
        }
        if let Some(user) = opts.user.as_ref() {
            config.insert("user".to_string(), user.to_string());
        }
        if !opts.identity_files.is_empty() {
            config.insert(
                "identityfile".to_string(),
                opts.identity_files
                    .iter()
                    .filter_map(|p| p.to_str())
                    .map(ToString::to_string)
                    .collect::<Vec<String>>()
                    .join(" "),
            );
        }
        if let Some(yes) = opts.identities_only.as_ref() {
            let value = if *yes {
                "yes".to_string()
            } else {
                "no".to_string()
            };
            config.insert("identitiesonly".to_string(), value);
        }
        if let Some(cmd) = opts.proxy_command.as_ref() {
            config.insert("proxycommand".to_string(), cmd.to_string());
        }
        if !opts.user_known_hosts_files.is_empty() {
            config.insert(
                "userknownhostsfile".to_string(),
                opts.user_known_hosts_files
                    .iter()
                    .filter_map(|p| p.to_str())
                    .map(ToString::to_string)
                    .collect::<Vec<String>>()
                    .join(" "),
            );
        }

        // Set verbosity optin for ssh lib
        config.insert("wezterm_ssh_verbose".to_string(), opts.verbose.to_string());

        // Set the backend to use going forward
        config.insert("wezterm_ssh_backend".to_string(), opts.backend.to_string());

        // Add in any of the other options provided
        config.extend(opts.other);

        // Port should always exist, otherwise WezSession will panic from unwrap()
        let port = config
            .get("port")
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Missing port"))?
            .parse::<u16>()
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;

        // Establish a connection
        trace!("WezSession::connect({:?})", config);
        let (session, events) =
            WezSession::connect(config).map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;

        Ok(Self {
            session,
            events,
            host: host.as_ref().to_string(),
            port,
            authenticated: false,
        })
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

        // Perform the authentication by listening for events and continuing to handle them
        // until authenticated
        while let Ok(event) = self.events.recv().await {
            match event {
                WezSessionEvent::Banner(banner) => {
                    if let Some(banner) = banner {
                        handler.on_banner(banner.as_ref()).await;
                    }
                }
                WezSessionEvent::HostVerify(verify) => {
                    let verified = handler.on_verify_host(verify.message.as_str()).await?;
                    verify
                        .answer(verified)
                        .compat()
                        .await
                        .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;
                }
                WezSessionEvent::Authenticate(mut auth) => {
                    let ev = SshAuthEvent {
                        username: auth.username.clone(),
                        instructions: auth.instructions.clone(),
                        prompts: auth
                            .prompts
                            .drain(..)
                            .map(|p| SshAuthPrompt {
                                prompt: p.prompt,
                                echo: p.echo,
                            })
                            .collect(),
                    };

                    let answers = handler.on_authenticate(ev).await?;
                    auth.answer(answers)
                        .compat()
                        .await
                        .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;
                }
                WezSessionEvent::Error(err) => {
                    handler.on_error(&err).await;
                    return Err(io::Error::new(io::ErrorKind::PermissionDenied, err));
                }
                WezSessionEvent::Authenticated => break,
            }
        }

        // Mark as authenticated
        self.authenticated = true;

        Ok(())
    }

    /// Consume [`Ssh`] and produce a [`DistantClient`] that is connected to a remote
    /// distant server that is spawned using the ssh client
    pub async fn launch_and_connect(self, opts: DistantLaunchOpts) -> io::Result<DistantClient> {
        trace!("ssh::launch_and_connect({:?})", opts);

        // Exit early if not authenticated as this is a requirement
        if !self.authenticated {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Not authenticated",
            ));
        }

        let timeout = opts.timeout;

        // Determine distinct candidate ip addresses for connecting
        //
        // NOTE: This breaks when the host is an alias defined within an ssh config; however,
        //       we need to be able to resolve the IP address(es) for use in TCP connect. The
        //       end solution would be to have wezterm-ssh provide some means to determine the
        //       IP address of the end machine it is connected to, but that probably isn't
        //       possible with ssh. So, for now, connecting to a distant server from an
        //       established ssh connection requires that we can resolve the specified host
        let mut candidate_ips = tokio::net::lookup_host(format!("{}:{}", self.host, self.port))
            .await
            .map_err(|x| {
                io::Error::new(
                    x.kind(),
                    format!("{} needs to be resolvable outside of ssh: {}", self.host, x),
                )
            })?
            .into_iter()
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
        let codec = XChaCha20Poly1305Codec::from(key);

        // Try each IP address with the same port to see if one works
        let mut err = None;
        for ip in candidate_ips {
            let addr = SocketAddr::new(ip, credentials.port);
            debug!("Attempting to connect to distant server @ {}", addr);
            match DistantClient::connect_timeout(addr, codec.clone(), timeout).await {
                Ok(client) => return Ok(client),
                Err(x) => err = Some(x),
            }
        }

        // If all failed, return the last error we got
        Err(err.expect("Err set above"))
    }

    /// Consume [`Ssh`] and launch a distant server, returning a [`DistantSingleKeyCredentials`]
    /// tied to the launched server that includes credentials
    pub async fn launch(self, opts: DistantLaunchOpts) -> io::Result<DistantSingleKeyCredentials> {
        trace!("ssh::launch({:?})", opts);

        // Exit early if not authenticated as this is a requirement
        if !self.authenticated {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Not authenticated",
            ));
        }

        let host = self.host().to_string();

        // Turn our ssh connection into a client/server pair so we can use it to spawn our server
        let (mut client, server) = self.into_distant_pair().await?;

        // Build arguments for distant to execute listen subcommand
        let mut args = vec![
            String::from("server"),
            String::from("listen"),
            String::from("--daemon"),
            String::from("--host"),
            String::from("ssh"),
        ];
        args.extend(
            shell_words::split(&opts.args)
                .map_err(|x| io::Error::new(io::ErrorKind::InvalidInput, x))?,
        );

        // If we are using a login shell, we need to make the binary be sh
        // so we can appropriately pipe into the login shell
        let cmd = if opts.use_login_shell {
            format!(
                "sh -c {}",
                shell_words::quote(&format!(
                    "echo {} {} | $SHELL -l",
                    opts.binary,
                    args.join(" ")
                ))
            )
        } else {
            format!("{} {}", opts.binary, args.join(" "))
        };

        // Spawn distant server and detach it so that we don't kill it when the
        // ssh client is closed
        debug!("Executing {}", cmd);
        let output = client.output(cmd, None).await?;
        debug!(
            "Completed with success = {}, code = {:?}",
            output.success, output.code
        );

        // Close out ssh client by killing the internal server and client
        server.abort();
        client.abort();
        let _ = client.wait().await;

        // If successful, grab the client information and establish a connection
        // with the distant server
        if output.success {
            // Iterate over output as individual lines, looking for client info
            trace!("Searching for credentials");
            let maybe_info = output
                .stdout
                .split(|&b| b == b'\n')
                .map(String::from_utf8_lossy)
                .find_map(|line| line.parse::<DistantSingleKeyCredentials>().ok());
            match maybe_info {
                Some(mut info) => {
                    info.host = host;
                    Ok(info)
                }
                None => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Missing launch information",
                )),
            }
        } else {
            Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "Spawning distant failed [{}]: {}",
                    output
                        .code
                        .map(|x| x.to_string())
                        .unwrap_or_else(|| String::from("???")),
                    match String::from_utf8(output.stderr) {
                        Ok(output) => output,
                        Err(x) => x.to_string(),
                    }
                ),
            ))
        }
    }

    /// Consume [`Ssh`] and produce a [`DistantClient`] that is powered by an ssh client
    /// underneath
    pub async fn into_distant_client(self) -> io::Result<DistantClient> {
        Ok(self.into_distant_pair().await?.0)
    }

    /// Consume [`Ssh`] and produce a [`BoxedDistantWriterReader`] that is powered by an ssh client
    /// underneath
    pub async fn into_distant_writer_reader(self) -> io::Result<BoxedDistantWriterReader> {
        Ok(self.into_writer_reader_and_server().await?.0)
    }

    /// Consumes [`Ssh`] and produces a [`DistantClient`] and [`DistantApiServer`] pair
    pub async fn into_distant_pair(self) -> io::Result<(DistantClient, Box<dyn ServerRef>)> {
        let ((writer, reader), server) = self.into_writer_reader_and_server().await?;
        let client = DistantClient::new(writer, reader)?;
        Ok((client, server))
    }

    /// Consumes [`Ssh`] and produces a [`DistantClient`] and [`DistantApiServer`] pair
    async fn into_writer_reader_and_server(
        self,
    ) -> io::Result<(BoxedDistantWriterReader, Box<dyn ServerRef>)> {
        // Exit early if not authenticated as this is a requirement
        if !self.authenticated {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Not authenticated",
            ));
        }

        let (t1, t2) = FramedTransport::pair(1);

        // Spawn a bridge client that is directly connected to our server
        let (writer, reader) = t1.into_split();
        let writer: BoxedDistantWriter = Box::new(writer);
        let reader: BoxedDistantReader = Box::new(reader);

        // Spawn a bridge server that is directly connected to our client
        let server = {
            let Self {
                session: wez_session,
                ..
            } = self;
            let (writer, reader) = t2.into_split();
            DistantApiServer::new(SshDistantApi::new(wez_session))
                .start(OneshotListener::from_value((writer, reader)))?
        };

        Ok(((writer, reader), server))
    }
}
