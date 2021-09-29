use async_compat::CompatExt;
use distant_core::{Request, Session, Transport};
use log::*;
use smol::channel::Receiver as SmolReceiver;
use std::{
    collections::BTreeMap,
    io::{self, Write},
    path::PathBuf,
    sync::Arc,
};
use tokio::sync::{mpsc, Mutex};
use wezterm_ssh::{Config as WezConfig, Session as WezSession, SessionEvent as WezSessionEvent};

mod handler;

#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Ssh2AuthPrompt {
    /// The label to show when prompting the user
    pub prompt: String,

    /// If true, the response that the user inputs should be displayed as they type. If false then
    /// treat it as a password entry and do not display what is typed in response to this prompt.
    pub echo: bool,
}

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

#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Ssh2SessionOpts {
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

    /// Additional options to provide as defined by `ssh_config(5)`
    pub other: BTreeMap<String, String>,
}

pub struct Ssh2AuthHandler {
    pub on_authenticate: Box<dyn FnMut(Ssh2AuthEvent) -> io::Result<Vec<String>>>,
    pub on_banner: Box<dyn FnMut(&str)>,
    pub on_host_verify: Box<dyn FnMut(&str) -> io::Result<bool>>,
    pub on_error: Box<dyn FnMut(&str)>,
}

impl Default for Ssh2AuthHandler {
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

pub struct Ssh2Session {
    session: WezSession,
    events: SmolReceiver<WezSessionEvent>,
}

impl Ssh2Session {
    /// Connect to a remote TCP server using SSH
    pub fn connect(host: impl AsRef<str>, opts: Ssh2SessionOpts) -> io::Result<Self> {
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

        // Add in any of the other options provided
        config.extend(opts.other);

        // Establish a connection
        let (session, events) =
            WezSession::connect(config).map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;

        Ok(Self { session, events })
    }

    /// Authenticates the [`Ssh2Session`] and produces a [`Session`]
    pub async fn authenticate(self, mut handler: Ssh2AuthHandler) -> io::Result<Session> {
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

        // We are now authenticated, so convert into a distant session that wraps our ssh2 session
        self.into_session()
    }

    /// Consume [`Ssh2Session`] and produce a distant [`Session`]
    fn into_session(self) -> io::Result<Session> {
        let (t1, t2) = Transport::pair(1);
        let session = Session::initialize(t1)?;

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
                    error!("{}", x);
                }
            }
        });

        tokio::spawn(async move {
            while let Some(res) = rx.recv().await {
                if t_write.send(res).await.is_err() {
                    break;
                }
            }
        });

        Ok(session)
    }
}
