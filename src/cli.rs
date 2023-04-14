use crate::{CliResult, Options};
use std::ffi::OsString;

mod cache;
mod client;
mod commands;
mod manager;
mod spawner;

pub(crate) use cache::Cache;
pub(crate) use client::Client;
use commands::DistantSubcommand;
pub(crate) use manager::Manager;

#[cfg_attr(unix, allow(unused_imports))]
pub(crate) use spawner::Spawner;

/// Represents the primary CLI entrypoint
pub struct Cli {
    options: Options,
}

impl Cli {
    /// Creates a new CLI instance by parsing command-line arguments
    pub fn initialize() -> anyhow::Result<Self> {
        Self::initialize_from(std::env::args_os())
    }

    /// Creates a new CLI instance by parsing providing arguments
    pub fn initialize_from<I, T>(args: I) -> anyhow::Result<Self>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        Ok(Cli {
            options: Options::load_from(args)?,
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
    pub fn run(self) -> CliResult {
        match self.command {
            DistantSubcommand::Client(cmd) => cmd.run(self.config.client),
            DistantSubcommand::Generate(cmd) => cmd.run(self.config.generate),
            DistantSubcommand::Manager(cmd) => cmd.run(self.config.manager),
            DistantSubcommand::Server(cmd) => cmd.run(self.config.server),
        }
    }
}
