use async_compat::CompatExt;
use distant_core::{Request, Session, Transport};
use log::*;
use smol::channel::Receiver as SmolReceiver;
use std::{
    io::{self, Write},
    sync::Arc,
};
use tokio::sync::{mpsc, Mutex};
use wezterm_ssh::{Config as WezConfig, Session as WezSession, SessionEvent as WezSessionEvent};

mod handler;

#[derive(Debug)]
pub struct Ssh2AuthPrompt {
    /// The label to show when prompting the user
    pub prompt: String,

    /// If true, the response that the user inputs should be displayed as they type. If false then
    /// treat it as a password entry and do not display what is typed in response to this prompt.
    pub echo: bool,
}

#[derive(Debug)]
pub struct Ssh2AuthEvent {
    /// Represents the name of the user to be authenticated. This may be empty!
    pub username: String,

    /// Informational text to be displayed to the user prior to the prompt
    pub instructions: String,

    /// Prompts to be conveyed to the user, each representing a single answer needed
    pub prompts: Vec<Ssh2AuthPrompt>,
}

#[derive(Clone, Debug, Default)]
pub struct Ssh2SessionOpts {
    pub port: Option<u16>,
    pub user: Option<String>,
}

pub struct Ssh2AuthHandler {
    on_authenticate: Box<dyn FnMut(Ssh2AuthEvent) -> io::Result<Vec<String>>>,
    on_banner: Box<dyn FnMut(&str)>,
    on_host_verify: Box<dyn FnMut(&str) -> io::Result<bool>>,
    on_error: Box<dyn FnMut(&str)>,
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
                match rpassword::prompt_password_stderr("Enter [y/n]> ")?.as_str() {
                    "y" | "Y" | "yes" | "YES" => Ok(true),
                    "n" | "N" | "no" | "NO" | _ => Ok(false),
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

        let mut config = config.for_host(host.as_ref());
        if let Some(port) = opts.port.as_ref() {
            config.insert("port".to_string(), port.to_string());
        }
        if let Some(user) = opts.user.as_ref() {
            config.insert("user".to_string(), user.to_string());
        }

        let (session, events) = WezSession::connect(config.clone())
            .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;

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
