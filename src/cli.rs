use crate::{
    config::{CommonConfig, Config},
    paths, ExitCode,
};
use clap::Parser;
use std::{ffi::OsString, path::PathBuf};

mod cache;
mod client;
mod commands;
mod error;
mod manager;
mod spawner;

pub(crate) use cache::Cache;
pub(crate) use client::Client;
use commands::DistantSubcommand;
pub use error::{CliError, CliResult};
pub(crate) use manager::Manager;

#[cfg(windows)]
pub(crate) use spawner::Spawner;

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

    /// Configuration file to load instead of the default paths
    #[clap(short = 'c', long = "config", global = true, value_parser)]
    config_path: Option<PathBuf>,

    #[clap(subcommand)]
    command: DistantSubcommand,
}

impl Cli {
    /// Creates a new CLI instance by parsing command-line arguments
    pub fn initialize() -> CliResult<Self> {
        Self::initialize_from(std::env::args_os())
    }

    /// Creates a new CLI instance by parsing providing arguments
    pub fn initialize_from<I, T>(args: I) -> CliResult<Self>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let Opt {
            mut common,
            config_path,
            command,
        } = Opt::try_parse_from(args).map_err(CliError::Usage)?;

        // Try to load a configuration file, defaulting if no config file is found
        let config = Config::load_multi(config_path).map_err(ExitCode::config_error)?;

        // Extract the common config from our config file
        let config_common = match &command {
            DistantSubcommand::Client(_) => config.client.common.clone(),
            DistantSubcommand::Manager(_) => config.manager.common.clone(),
            DistantSubcommand::Server(_) => config.server.common.clone(),
        };

        // Blend common configs together
        common.log_file = common.log_file.or(config_common.log_file);
        common.log_level = common.log_level.or(config_common.log_level);

        // Assign the appropriate log file based on client/manager/server
        if common.log_file.is_none() {
            // NOTE: We assume that any of these commands will log to the user-specific path
            //       and that services that run manager will explicitly override the
            //       log file path
            common.log_file = Some(match &command {
                DistantSubcommand::Client(_) => paths::user::CLIENT_LOG_FILE_PATH.to_path_buf(),
                DistantSubcommand::Server(_) => paths::user::SERVER_LOG_FILE_PATH.to_path_buf(),

                // If we are listening as a manager, then we want to log to a manager-specific file
                DistantSubcommand::Manager(cmd) if cmd.is_listen() => {
                    paths::user::MANAGER_LOG_FILE_PATH.to_path_buf()
                }

                // Otherwise, if we are performing some operation as a client talking to the
                // manager, then we want to log to the client file
                DistantSubcommand::Manager(_) => paths::user::CLIENT_LOG_FILE_PATH.to_path_buf(),
            });
        }

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
        }

        // Create our logger, but don't initialize yet
        let logger = Logger::with(builder.build()).format_for_files(flexi_logger::opt_format);

        // Assign our log output to a file
        // NOTE: We can unwrap here as we assign the log file earlier
        let logger = logger.log_to_file(
            FileSpec::try_from(self.common.log_file.as_ref().unwrap())
                .expect("Failed to create log file spec"),
        );

        logger.start().expect("Failed to initialize logger")
    }

    #[cfg(windows)]
    pub fn is_manager_listen_command(&self) -> bool {
        match &self.command {
            DistantSubcommand::Manager(cmd) => cmd.is_listen(),
            _ => false,
        }
    }

    /// Runs the CLI
    pub fn run(self) -> CliResult<()> {
        match self.command {
            DistantSubcommand::Client(cmd) => cmd.run(self.config.client),
            DistantSubcommand::Manager(cmd) => cmd.run(self.config.manager),
            DistantSubcommand::Server(cmd) => cmd.run(self.config.server),
        }
    }
}
