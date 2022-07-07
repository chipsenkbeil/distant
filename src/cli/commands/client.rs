use crate::{
    cli::{
        client::{MsgReceiver, MsgSender},
        CliError, CliResult, Client, Storage,
    },
    config::{ClientConfig, ClientLaunchConfig},
};
use clap::Subcommand;
use distant_core::{
    ConnectionId, Destination, DistantManagerClient, DistantMsg, DistantRequestData, Extra,
};
use log::*;
use std::time::Duration;

mod buf;
mod format;
mod link;
mod lsp;
mod shell;
mod stdin;

pub use format::Format;
use format::Formatter;
use lsp::Lsp;
use shell::Shell;

#[derive(Debug, Subcommand)]
pub enum ClientSubcommand {
    /// Performs some action on a remote machine
    Action {
        /// Specify a connection being managed
        #[clap(long)]
        connection: Option<ConnectionId>,

        /// Represents the maximum time (in seconds) to wait for a network request before timing out
        #[clap(short, long)]
        timeout: Option<f32>,

        #[clap(subcommand)]
        request: DistantRequestData,
    },

    /// Launches the server-portion of the binary on a remote machine
    Launch {
        #[clap(flatten)]
        config: ClientLaunchConfig,

        #[clap(short, long, value_enum)]
        format: Format,

        destination: Box<Destination>,
    },

    /// Specialized treatment of running a remote LSP process
    Lsp {
        /// Specify a connection being managed
        #[clap(long)]
        connection: Option<ConnectionId>,

        /// If provided, will run in persist mode, meaning that the process will not be killed if the
        /// client disconnects from the server
        #[clap(long)]
        persist: bool,

        /// If provided, will run LSP in a pty
        #[clap(long)]
        pty: bool,

        cmd: String,
    },

    /// Runs actions in a read-eval-print loop
    Repl {
        /// Specify a connection being managed
        #[clap(long)]
        connection: Option<ConnectionId>,

        /// Format used for input into and output from the repl
        #[clap(short, long, value_enum)]
        format: Format,

        /// Represents the maximum time (in seconds) to wait for a network request before timing out
        #[clap(short, long)]
        timeout: Option<f32>,
    },

    /// Specialized treatment of running a remote shell process
    Shell {
        /// Specify a connection being managed
        #[clap(long)]
        connection: Option<ConnectionId>,

        /// If provided, will run in persist mode, meaning that the process will not be killed if the
        /// client disconnects from the server
        #[clap(long)]
        persist: bool,

        /// Optional command to run instead of $SHELL
        cmd: Option<String>,
    },
}

impl ClientSubcommand {
    pub fn run(self, config: ClientConfig) -> CliResult<()> {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(Self::async_run(self, config))
    }

    async fn async_run(self, config: ClientConfig) -> CliResult<()> {
        match self {
            Self::Action {
                connection,
                request,
                timeout,
            } => {
                debug!("Connecting to manager: {:?}", config.network.as_os_str());
                let mut client = Client::new(config.network).connect().await?;

                let connection_id = use_or_lookup_connection_id(connection, &mut client).await?;

                debug!("Opening channel to connection {}", connection_id);
                let mut channel = client.open_channel(connection_id).await?;

                debug!(
                    "Timeout configured to be {}",
                    match timeout {
                        Some(secs) => format!("{}s", secs),
                        None => "none".to_string(),
                    }
                );
                debug!("Sending request {:?}", request);
                let response = channel
                    .send_timeout(
                        DistantMsg::Single(request),
                        timeout
                            .or(config.action.timeout)
                            .map(Duration::from_secs_f32),
                    )
                    .await?;

                debug!("Got response {:?}", response);
                Formatter::new(Format::Shell).print(response)?;
            }
            Self::Launch {
                config: launcher_config,
                format,
                destination,
            } => {
                debug!("Connecting to manager: {:?}", config.network.as_os_str());
                let mut client = {
                    let client = match format {
                        Format::Shell => Client::new(config.network),
                        Format::Json => Client::new(config.network).using_msg_stdin_stdout(),
                    };
                    client.connect().await?
                };

                // Merge our launch configs, overwriting anything in the config file
                // with our cli arguments
                let mut extra = Extra::from(config.launch);
                extra.extend(Extra::from(launcher_config).into_map());

                // Start the server using our manager
                debug!("Launching server at {} with {}", destination, extra);
                let destination = client.launch(*destination, extra).await?;

                // Trigger our manager to connect to the launched server
                debug!("Connecting to server at {}", destination);
                let id = client.connect(destination, Extra::new()).await?;

                // Mark the server's id as the new default
                debug!("Updating cached default connection id to {}", id);
                let mut storage = Storage::read_or_default().await?;
                storage.default_connection_id = id;
                storage.write().await?;
            }
            Self::Lsp {
                connection,
                persist,
                pty,
                cmd,
            } => {
                debug!("Connecting to manager: {:?}", config.network.as_os_str());
                let mut client = Client::new(config.network).connect().await?;

                let connection_id = use_or_lookup_connection_id(connection, &mut client).await?;

                debug!("Opening channel to connection {}", connection_id);
                let channel = client.open_channel(connection_id).await?;

                debug!(
                    "Spawning LSP server (persist = {}, pty = {}): {}",
                    persist, pty, cmd
                );
                Lsp::new(channel).spawn(cmd, persist, pty).await?;
            }
            Self::Repl {
                connection,
                format,
                timeout,
            } => {
                debug!("Connecting to manager: {:?}", config.network.as_os_str());
                let mut client = Client::new(config.network)
                    .using_msg_stdin_stdout()
                    .connect()
                    .await?;

                let connection_id = use_or_lookup_connection_id(connection, &mut client).await?;

                debug!("Opening channel to connection {}", connection_id);
                let mut channel = client.open_channel(connection_id).await?;

                debug!(
                    "Timeout configured to be {}",
                    match timeout {
                        Some(secs) => format!("{}s", secs),
                        None => "none".to_string(),
                    }
                );

                debug!("Starting repl using format {:?}", format);
                let tx = MsgSender::from_stdout();
                let mut rx = MsgReceiver::from_stdin().into_rx();
                loop {
                    match rx.recv().await {
                        Some(Ok(request)) => {
                            debug!("Sending request {:?}", request);
                            let response = channel
                                .send_timeout(
                                    DistantMsg::Single(request),
                                    timeout.or(config.repl.timeout).map(Duration::from_secs_f32),
                                )
                                .await?;

                            debug!("Got response {:?}", response);
                            tx.send_blocking(&response)?;
                        }
                        Some(Err(x)) => error!("{}", x),
                        None => {
                            debug!("Shutting down repl");
                            break;
                        }
                    }
                }
            }
            Self::Shell {
                connection,
                persist,
                cmd,
            } => {
                debug!("Connecting to manager: {:?}", config.network.as_os_str());
                let mut client = Client::new(config.network).connect().await?;

                let connection_id = use_or_lookup_connection_id(connection, &mut client).await?;

                debug!("Opening channel to connection {}", connection_id);
                let channel = client.open_channel(connection_id).await?;

                debug!(
                    "Spawning shell (persist = {}): {}",
                    persist,
                    cmd.as_deref().unwrap_or(r"$SHELL")
                );
                Shell::new(channel).spawn(cmd, persist).await?;
            }
        }

        Ok(())
    }
}

async fn use_or_lookup_connection_id(
    connection: Option<ConnectionId>,
    client: &mut DistantManagerClient,
) -> CliResult<ConnectionId> {
    match connection {
        Some(id) => {
            trace!("Using specified connection id: {}", id);
            Ok(id)
        }
        None => {
            trace!("Looking up connection id");
            let mut storage = Storage::read_or_default().await?;
            let list = client.list().await?;

            if list.contains_key(&storage.default_connection_id) {
                trace!(
                    "Using cached connection id: {}",
                    storage.default_connection_id
                );
                Ok(storage.default_connection_id)
            } else if list.is_empty() {
                trace!("Cached connection id is invalid as there are no connections");
                Err(CliError::NoConnection)
            } else if list.len() > 1 {
                trace!("Cached connection id is invalid and there are multiple connections");
                Err(CliError::NeedToPickConnection)
            } else {
                trace!("Cached connection id is invalid");
                storage.default_connection_id = *list.keys().next().unwrap();
                trace!(
                    "Detected singular connection id, so updating cache: {}",
                    storage.default_connection_id
                );
                storage.write().await?;
                Ok(storage.default_connection_id)
            }
        }
    }
}
