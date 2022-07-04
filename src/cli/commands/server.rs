use crate::{
    cli::CliResult,
    config::{ServerConfig, ServerListenConfig, ServiceKind},
    Merge,
};
use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum ServerSubcommand {
    /// Start the server as a service
    Start {
        /// Type of service manager used to run this service
        #[clap(value_enum)]
        kind: ServiceKind,
    },

    /// Stop the server as a service
    Stop,

    /// Install the server as a service
    Install {
        #[clap(value_enum)]
        kind: ServiceKind,
    },

    /// Uninstall the server as a service
    Uninstall {
        #[clap(value_enum)]
        kind: ServiceKind,
    },

    /// Listen for incoming requests as a server
    Listen {
        #[clap(flatten)]
        config: ServerListenConfig,

        /// If specified, will fork the process to run as a standalone daemon
        #[clap(long)]
        daemon: bool,

        /// If specified, the server will not generate a key but instead listen on stdin for the next
        /// 32 bytes that it will use as the key instead. Receiving less than 32 bytes before stdin
        /// is closed is considered an error and any bytes after the first 32 are not used for the key
        #[clap(long)]
        key_from_stdin: bool,
    },
}

impl ServerSubcommand {
    pub async fn run(self, config: ServerConfig) -> CliResult<()> {
        match self {
            Self::Start { kind } => todo!(),
            Self::Stop => todo!(),

            Self::Install { kind } => todo!(),
            Self::Uninstall { kind } => todo!(),

            Self::Listen { .. } => todo!(),
        }

        Ok(())
    }
}
