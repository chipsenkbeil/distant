use async_compat::CompatExt;
use distant_core::{
    Request, Session, SessionChannelExt, SessionDetails, SessionInfo, Transport,
    XChaCha20Poly1305Codec,
};
use log::*;
use smol::channel::Receiver as SmolReceiver;
use std::{
    collections::BTreeMap,
    fmt,
    io::{self, Write},
    net::{IpAddr, SocketAddr},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::sync::{mpsc, Mutex};
use wezterm_ssh::{Config as WezConfig, Session as WezSession, SessionEvent as WezSessionEvent};

mod handler;
mod process;

/// Represents the backend to use for ssh operations
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
pub enum SshBackend {
    /// Use libssh as backend
    LibSsh,

    /// Use ssh2 as backend
    Ssh2,
}

impl Default for SshBackend {
    /// Defaults to libssh
    fn default() -> Self {
        Self::Ssh2
    }
}

impl fmt::Display for SshBackend {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::LibSsh => write!(f, "libssh"),
            Self::Ssh2 => write!(f, "ssh2"),
        }
    }
}

/// Represents a singular authentication prompt for a new ssh session
#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Ssh2AuthPrompt {
    /// The label to show when prompting the user
    pub prompt: String,

    /// If true, the response that the user inputs should be displayed as they type. If false then
    /// treat it as a password entry and do not display what is typed in response to this prompt.
    pub echo: bool,
}

/// Represents an authentication request that needs to be handled before an ssh session can be
/// established
#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Ssh2AuthEvent {
    /// Represents the name of the user to be authenticated. This may be empty!
    pub username: String,

    /// Informational text to be displayed to the user prior to the prompt
    pub instructions: String,

    /// Prompts to be conveyed to the user, each representing a single answer needed
    pub prompts: Vec<Ssh2AuthPrompt>,
}

/// Represents options to be provided when establishing an ssh session
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Ssh2SessionOpts {
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

/// Represents options to be provided when converting an ssh session into a distant session
#[derive(Clone, Debug)]
pub struct IntoDistantSessionOpts {
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

impl Default for IntoDistantSessionOpts {
    fn default() -> Self {
        Self {
            binary: String::from("distant"),
            args: String::new(),
            use_login_shell: false,
            timeout: Duration::from_secs(15),
        }
    }
}

/// Represents callback functions to be invoked during authentication of an ssh session
pub struct Ssh2AuthHandler<'a> {
    /// Invoked whenever a series of authentication prompts need to be displayed and responded to,
    /// receiving one event at a time and returning a collection of answers matching the total
    /// prompts provided in the event
    pub on_authenticate: Box<dyn FnMut(Ssh2AuthEvent) -> io::Result<Vec<String>> + 'a>,

    /// Invoked when receiving a banner from the ssh server, receiving the banner as a str, useful
    /// to display to the user
    pub on_banner: Box<dyn FnMut(&str) + 'a>,

    /// Invoked when the host is unknown for a new ssh connection, receiving the host as a str and
    /// returning true if the host is acceptable or false if the host (and thereby ssh session)
    /// should be declined
    pub on_host_verify: Box<dyn FnMut(&str) -> io::Result<bool> + 'a>,

    /// Invoked when an error is encountered, receiving the error as a str
    pub on_error: Box<dyn FnMut(&str) + 'a>,
}

impl Default for Ssh2AuthHandler<'static> {
    fn default() -> Self {
        Self {
            on_authenticate: Box::new(|ev| {
                if !ev.username.is_empty() {
                    eprintln!("Authentication for {}", ev.username);
                }

                if !ev.instructions.is_empty() {
                    eprintln!("{}", ev.instructions);
                }

                let mut answers = Vec::new();
                for prompt in &ev.prompts {
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
                        rpassword::prompt_password_stderr(prompt_line)?
                    };

                    answers.push(answer);
                }
                Ok(answers)
            }),
            on_banner: Box::new(|_| {}),
            on_host_verify: Box::new(|message| {
                eprintln!("{}", message);
                match rpassword::prompt_password_stderr("Enter [y/N]> ")?.as_str() {
                    "y" | "Y" | "yes" | "YES" => Ok(true),
                    _ => Ok(false),
                }
            }),
            on_error: Box::new(|_| {}),
        }
    }
}

/// Represents an ssh2 session
pub struct Ssh2Session {
    session: WezSession,
    events: SmolReceiver<WezSessionEvent>,
    host: String,
    port: u16,
    authenticated: bool,
}

impl Ssh2Session {
    /// Connect to a remote TCP server using SSH
    pub fn connect(host: impl AsRef<str>, opts: Ssh2SessionOpts) -> io::Result<Self> {
        debug!(
            "Establishing ssh connection to {} using {:?}",
            host.as_ref(),
            opts
        );
        let mut config = WezConfig::new();
        config.add_default_config_files();

        // Grab the config for the specific host
        let mut config = config.for_host(host.as_ref());

        // Override config with any settings provided by session opts
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

    /// Host this session is connected to
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Port this session is connected to on remote host
    pub fn port(&self) -> u16 {
        self.port
    }

    #[inline]
    pub fn is_authenticated(&self) -> bool {
        self.authenticated
    }

    /// Authenticates the [`Ssh2Session`] if not already authenticated
    pub async fn authenticate(&mut self, mut handler: Ssh2AuthHandler<'_>) -> io::Result<()> {
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
                        (handler.on_banner)(banner.as_ref());
                    }
                }
                WezSessionEvent::HostVerify(verify) => {
                    let verified = (handler.on_host_verify)(verify.message.as_str())?;
                    verify
                        .answer(verified)
                        .compat()
                        .await
                        .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;
                }
                WezSessionEvent::Authenticate(mut auth) => {
                    let ev = Ssh2AuthEvent {
                        username: auth.username.clone(),
                        instructions: auth.instructions.clone(),
                        prompts: auth
                            .prompts
                            .drain(..)
                            .map(|p| Ssh2AuthPrompt {
                                prompt: p.prompt,
                                echo: p.echo,
                            })
                            .collect(),
                    };

                    let answers = (handler.on_authenticate)(ev)?;
                    auth.answer(answers)
                        .compat()
                        .await
                        .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;
                }
                WezSessionEvent::Error(err) => {
                    (handler.on_error)(&err);
                    return Err(io::Error::new(io::ErrorKind::PermissionDenied, err));
                }
                WezSessionEvent::Authenticated => break,
            }
        }

        // Mark as authenticated
        self.authenticated = true;

        Ok(())
    }

    /// Consume [`Ssh2Session`] and produce a distant [`Session`] that is connected to a remote
    /// distant server that is spawned using the ssh session
    pub async fn into_distant_session(self, opts: IntoDistantSessionOpts) -> io::Result<Session> {
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

        let info = self.into_distant_session_info(opts).await?;
        let key = info.key;
        let codec = XChaCha20Poly1305Codec::from(key);

        // Try each IP address with the same port to see if one works
        let mut err = None;
        for ip in candidate_ips {
            let addr = SocketAddr::new(ip, info.port);
            debug!("Attempting to connect to distant server @ {}", addr);
            match Session::tcp_connect_timeout(addr, codec.clone(), timeout).await {
                Ok(session) => return Ok(session),
                Err(x) => err = Some(x),
            }
        }

        // If all failed, return the last error we got
        Err(err.expect("Err set above"))
    }

    /// Consume [`Ssh2Session`] and produce a distant [`SessionInfo`] representing a remote
    /// distant server that is spawned using the ssh session
    pub async fn into_distant_session_info(
        self,
        opts: IntoDistantSessionOpts,
    ) -> io::Result<SessionInfo> {
        // Exit early if not authenticated as this is a requirement
        if !self.authenticated {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Not authenticated",
            ));
        }

        let host = self.host().to_string();

        // Turn our ssh connection into a client session so we can use it to spawn our server
        let mut session = self.into_ssh_client_session().await?;

        // Build arguments for distant to execute listen subcommand
        let mut args = vec![
            String::from("listen"),
            String::from("--host"),
            String::from("ssh"),
        ];
        args.extend(
            shell_words::split(&opts.args)
                .map_err(|x| io::Error::new(io::ErrorKind::InvalidInput, x))?,
        );

        // If we are using a login shell, we need to make the binary be sh
        // so we can appropriately pipe into the login shell
        let (bin, args) = if opts.use_login_shell {
            (
                String::from("sh"),
                vec![
                    String::from("-c"),
                    shell_words::quote(&format!(
                        "echo {} {} | $SHELL -l",
                        opts.binary,
                        args.join(" ")
                    ))
                    .to_string(),
                ],
            )
        } else {
            (opts.binary, args)
        };

        // Spawn distant server and detach it so that we don't kill it when the
        // ssh session is closed
        debug!("Executing {} {}", bin, args.join(" "));
        let mut proc = session
            .spawn("<ssh-launch>", bin, args, true, None)
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;
        let mut stdout = proc.stdout.take().unwrap();
        let mut stderr = proc.stderr.take().unwrap();
        let (success, code) = proc
            .wait()
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::BrokenPipe, x))?;

        // Close out ssh session
        session.abort();
        let _ = session.wait().await;
        let mut output = Vec::new();

        // If successful, grab the session information and establish a connection
        // with the distant server
        if success {
            while let Ok(data) = stdout.read().await {
                output.extend(&data);
            }

            // Iterate over output as individual lines, looking for session info
            let maybe_info = output
                .split(|&b| b == b'\n')
                .map(|bytes: &[u8]| String::from_utf8_lossy(bytes))
                .find_map(|line| line.parse::<SessionInfo>().ok());
            match maybe_info {
                Some(mut info) => {
                    info.host = host;
                    Ok(info)
                }
                None => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Missing session data",
                )),
            }
        } else {
            while let Ok(data) = stderr.read().await {
                output.extend(&data);
            }

            Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "Spawning distant failed [{}]: {}",
                    code.map(|x| x.to_string())
                        .unwrap_or_else(|| String::from("???")),
                    match String::from_utf8(output) {
                        Ok(output) => output,
                        Err(x) => x.to_string(),
                    }
                ),
            ))
        }
    }

    /// Consume [`Ssh2Session`] and produce a distant [`Session`] that is powered by an ssh client
    /// underneath
    pub async fn into_ssh_client_session(self) -> io::Result<Session> {
        // Exit early if not authenticated as this is a requirement
        if !self.authenticated {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Not authenticated",
            ));
        }

        let (t1, t2) = Transport::pair(1);
        let tag = format!("ssh {}:{}", self.host, self.port);
        let session = Session::initialize_with_details(t1, Some(SessionDetails::Custom { tag }))?;

        // Spawn tasks that forward requests to the ssh session
        // and send back responses from the ssh session
        let (mut t_read, mut t_write) = t2.into_split();
        let Self {
            session: wez_session,
            ..
        } = self;

        let (tx, mut rx) = mpsc::channel(1);
        tokio::spawn(async move {
            let state = Arc::new(Mutex::new(handler::State::default()));
            while let Ok(Some(req)) = t_read.receive::<Request>().await {
                if let Err(x) =
                    handler::process(wez_session.clone(), Arc::clone(&state), req, tx.clone()).await
                {
                    error!("Ssh session receiver handler failed: {}", x);
                }
            }
            debug!("Ssh receiver task is now closed");
        });

        tokio::spawn(async move {
            while let Some(res) = rx.recv().await {
                if let Err(x) = t_write.send(res).await {
                    error!("Ssh session sender failed: {}", x);
                    break;
                }
            }
            debug!("Ssh sender task is now closed");
        });

        Ok(session)
    }
}
