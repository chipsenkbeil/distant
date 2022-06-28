use crate::{
    config::{CommonConfig, Config},
    constants::CONFIG_FILE_PATH,
};
use clap::{Parser, Subcommand};
use merge::Merge;
use std::{io, path::PathBuf};

mod action;
mod launch;
mod listen;
mod lsp;
mod manager;
mod repl;
mod shell;

/// Represents the primary CLI entrypoint
pub struct Cli {
    command: DistantSubcommand,
    config: Config,
}

impl Cli {
    /// Creates a new CLI instance by parsing command-line arguments
    pub async fn initialize() -> io::Result<Self> {
        let Opt {
            common,
            config_path,
            command,
        } = Opt::try_parse().map_err(|x| io::Error::new(io::ErrorKind::InvalidInput, x))?;

        let mut config = Config::load_from_file(config_path.as_path()).await?;
        config.client.common.merge(common);

        Ok(Cli { command, config })
    }

    /// Initializes a logger for the CLI, returning a handle to the logger
    pub fn init_logger(&self) -> flexi_logger::LoggerHandle {
        use flexi_logger::{FileSpec, LevelFilter, LogSpecification, Logger};
        let modules = &["distant", "distant_core", "distant_net", "distant_ssh2"];

        // Disable logging for everything but our binary, which is based on verbosity
        let mut builder = LogSpecification::builder();
        builder.default(LevelFilter::Off);

        // For each module, configure logging
        for module in modules {
            builder.module(
                module,
                self.config
                    .log_level
                    .unwrap_or_default()
                    .to_log_level_filter(),
            );

            // If quiet, we suppress all logging output
            //
            // NOTE: For a process request, unless logging to a file, we also suppress logging output
            //       to avoid unexpected results when being treated like a process
            //
            //       Without this, CI tests can sporadically fail when getting the exit code of a
            //       process because an error log is provided about failing to broadcast a response
            //       on the client side
            if self.config.quiet || (self.is_remote_process() && self.config.log_file.is_none()) {
                builder.module(module, LevelFilter::Off);
            }
        }

        // Create our logger, but don't initialize yet
        let logger = Logger::with(builder.build()).format_for_files(flexi_logger::opt_format);

        // If provided, log to file instead of stderr
        let logger = if let Some(path) = self.config.log_file.as_ref() {
            logger.log_to_file(FileSpec::try_from(path).expect("Failed to create log file spec"))
        } else {
            logger
        };

        logger.start().expect("Failed to initialize logger")
    }

    /// Runs the CLI
    pub async fn run(self) -> io::Result<()> {
        match self.command {
            DistantSubcommand::Action(cmd) => cmd.run(self.config).await,
            DistantSubcommand::Launch(cmd) => cmd.run(self.config).await,
            DistantSubcommand::Listen(cmd) => cmd.run(self.config).await,
            DistantSubcommand::Lsp(cmd) => cmd.run(self.config).await,
            DistantSubcommand::Manager(cmd) => cmd.run(self.config).await,
            DistantSubcommand::Repl(cmd) => cmd.run(self.config).await,
            DistantSubcommand::Shell(cmd) => cmd.run(self.config).await,
        }
    }

    fn is_remote_process(&self) -> bool {
        false
    }
}

#[derive(Debug, Parser)]
#[clap(name = "distant")]
struct Opt {
    #[clap(flatten)]
    common: CommonConfig,

    /// Configuration file to load
    #[clap(short, long = "config", value_parser, default_value_os_t = CONFIG_FILE_PATH.to_path_buf())]
    config_path: PathBuf,

    #[clap(subcommand)]
    command: DistantSubcommand,
}

#[derive(Debug, Subcommand)]
enum DistantSubcommand {
    /// Performs some action on a remote machine
    Action(action::Subcommand),

    /// Launches the server-portion of the binary on a remote machine
    Launch(launch::Subcommand),

    /// Begins listening for incoming requests
    Listen(listen::Subcommand),

    /// Specialized treatment of running a remote LSP process
    Lsp(lsp::Subcommand),

    /// Perform manager commands
    Manager(manager::Subcommand),

    /// Runs actions in a read-eval-print loop
    Repl(repl::Subcommand),

    /// Specialized treatment of running a remote shell process
    Shell(shell::Subcommand),
}
