use crate::{
    config::{CommonConfig, Config, Merge},
    constants::CONFIG_FILE_PATH,
};
use clap::Parser;
use std::{io, path::PathBuf};

mod client;
mod commands;
mod error;

pub(crate) use client::Client;
use commands::DistantSubcommand;
pub use error::{CliError, CliResult};

/// Represents the primary CLI entrypoint
pub struct Cli {
    common: CommonConfig,
    command: DistantSubcommand,
    config: Config,
}

#[derive(Debug, Parser)]
#[clap(author, version, about)]
#[clap(name = "distant")]
struct Opt {
    #[clap(flatten)]
    common: CommonConfig,

    /// Configuration file to load
    #[clap(
        short = 'c', 
        long = "config", 
        global = true, 
        value_parser, 
        default_value_os_t = CONFIG_FILE_PATH.to_path_buf()
    )]
    config_path: PathBuf,

    #[clap(subcommand)]
    command: DistantSubcommand,
}

impl Cli {
    /// Creates a new CLI instance by parsing command-line arguments
    pub fn initialize() -> CliResult<Self> {
        let Opt {
            common,
            config_path,
            command,
        } = Opt::try_parse().map_err(|x| io::Error::new(io::ErrorKind::InvalidInput, x))?;

        // Try to load a configuration file, defaulting if no config file is found
        let mut config = match Config::blocking_load_from_file(config_path.as_path()) {
            Ok(config) => config,
            Err(x) if x.kind() == io::ErrorKind::NotFound => Config::default(),
            Err(x) => return Err(x.into()),
        };

        // Update the common configuration based on our cli
        config.merge(common);

        let common = match &command {
            DistantSubcommand::Client(_) => config.client.common.clone(),
            DistantSubcommand::Manager(_) => config.manager.common.clone(),
            DistantSubcommand::Server(_) => config.server.common.clone(),
        };

        Ok(Cli {
            common,
            command,
            config,
        })
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
                self.common
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
            if self.common.quiet || (self.is_remote_process() && self.common.log_file.is_none()) {
                builder.module(module, LevelFilter::Off);
            }
        }

        // Create our logger, but don't initialize yet
        let logger = Logger::with(builder.build()).format_for_files(flexi_logger::opt_format);

        // If provided, log to file instead of stderr
        let logger = if let Some(path) = self.common.log_file.as_ref() {
            logger.log_to_file(FileSpec::try_from(path).expect("Failed to create log file spec"))
        } else {
            logger
        };

        logger.start().expect("Failed to initialize logger")
    }

    /// Runs the CLI
    pub fn run(self) -> CliResult<()> {
        match self.command {
            DistantSubcommand::Client(cmd) => cmd.run(self.config.client),
            DistantSubcommand::Manager(cmd) => cmd.run(self.config.manager),
            DistantSubcommand::Server(cmd) => cmd.run(self.config.server),
        }
    }

    fn is_remote_process(&self) -> bool {
        match &self.command {
            DistantSubcommand::Client(cmd) => cmd.is_remote_process(),
            _ => false,
        }
    }
}
