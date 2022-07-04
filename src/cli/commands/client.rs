use crate::{
    cli::{
        client::{MsgReceiver, MsgSender},
        CliResult, Client,
    },
    config::{ClientConfig, ClientLaunchConfig},
};
use clap::Subcommand;
use distant_core::{Destination, DistantMsg, DistantRequestData};
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
        #[clap(subcommand)]
        request: DistantRequestData,
    },

    /// Launches the server-portion of the binary on a remote machine
    Launch {
        #[clap(flatten)]
        config: ClientLaunchConfig,

        destination: Destination,
    },

    /// Specialized treatment of running a remote LSP process
    Lsp {
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
        #[clap(short, long, value_enum)]
        format: Format,
    },

    /// Specialized treatment of running a remote shell process
    Shell {
        /// If provided, will run in persist mode, meaning that the process will not be killed if the
        /// client disconnects from the server
        #[clap(long)]
        persist: bool,

        /// Optional command to run instead of $SHELL
        cmd: Option<String>,
    },
}

impl ClientSubcommand {
    pub fn is_remote_process(&self) -> bool {
        match self {
            Self::Action { request } => request.is_proc_spawn(),
            Self::Lsp { .. } | Self::Shell { .. } => true,
            _ => false,
        }
    }

    pub async fn run(self, config: ClientConfig) -> CliResult<()> {
        match self {
            Self::Action { request } => {
                let mut client = Client::new(config.network).connect().await?;
                let mut channel = client.open_channel(1).await?;
                let response = channel
                    .send_timeout(
                        DistantMsg::Single(request),
                        config.common.timeout.map(Duration::from_secs_f32),
                    )
                    .await?;

                Formatter::new(Format::Shell).print(response)?;
            }
            Self::Launch {
                config,
                destination,
            } => todo!(),
            Self::Lsp { persist, pty, cmd } => {
                let mut client = Client::new(config.network).connect().await?;
                let channel = client.open_channel(1).await?;
                Lsp::new(channel).spawn(cmd, persist, pty).await?;
            }
            Self::Repl { format } => {
                let mut client = Client::new(config.network)
                    .using_msg_stdin_stdout()
                    .connect()
                    .await?;
                let mut channel = client.open_channel(1).await?;

                let tx = MsgSender::from_stdout();
                let mut rx = MsgReceiver::from_stdin().into_rx();
                while let Some(Ok(request)) = rx.recv().await {
                    let response = channel
                        .send_timeout(
                            DistantMsg::Single(request),
                            config.common.timeout.map(Duration::from_secs_f32),
                        )
                        .await?;
                    tx.send_blocking(&response)?;
                }
            }
            Self::Shell { persist, cmd } => {
                let mut client = Client::new(config.network).connect().await?;
                let channel = client.open_channel(1).await?;
                Shell::new(channel).spawn(cmd, persist).await?;
            }
        }

        Ok(())
    }
}
