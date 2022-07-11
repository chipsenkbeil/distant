use crate::{
    config::{CommonConfig, Config},
    paths::{
        global as global_paths, user as user_paths, CLIENT_LOG_FILE_PATH, CONFIG_FILE_PATH,
        MANAGER_LOG_FILE_PATH, SERVER_LOG_FILE_PATH,
    },
};
use clap::Parser;
use std::{io, path::PathBuf};

mod client;
mod commands;
mod error;
mod manager;
mod service;
mod spawner;
mod storage;

pub(crate) use client::Client;
use commands::DistantSubcommand;
pub use error::{CliError, CliResult};
pub(crate) use manager::Manager;
pub(crate) use service::*;
pub(crate) use storage::Storage;

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
            mut common,
            config_path,
            command,
        } = Opt::try_parse().map_err(|x| io::Error::new(io::ErrorKind::InvalidInput, x))?;

        // Try to load a configuration file, defaulting if no config file is found
        let config = match Config::blocking_load_from_file(config_path.as_path()) {
            Ok(config) => config,
            Err(x) if x.kind() == io::ErrorKind::NotFound => Config::default(),
            Err(x) => return Err(x.into()),
        };

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
            common.log_file = Some(match &command {
                DistantSubcommand::Client(_) => CLIENT_LOG_FILE_PATH.to_path_buf(),
                DistantSubcommand::Manager(_) => MANAGER_LOG_FILE_PATH.to_path_buf(),
                DistantSubcommand::Server(_) => SERVER_LOG_FILE_PATH.to_path_buf(),
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

    /// Runs the CLI
    pub fn run(self) -> CliResult<()> {
        match self.command {
            DistantSubcommand::Client(cmd) => cmd.run(self.config.client),
            DistantSubcommand::Manager(cmd) => cmd.run(self.config.manager),
            DistantSubcommand::Server(cmd) => cmd.run(self.config.server),
        }
    }
}
