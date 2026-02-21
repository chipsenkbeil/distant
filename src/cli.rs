use std::ffi::OsString;

use crate::options::{DistantSubcommand, OptionsError};
use crate::{CliResult, Options};

mod commands;
mod common;

pub(crate) use common::Manager;
#[cfg_attr(unix, allow(unused_imports))]
pub(crate) use common::Spawner;

/// Represents the primary CLI entrypoint
#[derive(Debug)]
pub struct Cli {
    pub options: Options,
}

impl Cli {
    /// Creates a new CLI instance by parsing command-line arguments
    pub fn initialize() -> Result<Self, OptionsError> {
        Self::initialize_from(std::env::args_os())
    }

    /// Creates a new CLI instance by parsing providing arguments
    pub fn initialize_from<I, T>(args: I) -> Result<Self, OptionsError>
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
                self.options
                    .logging
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
            FileSpec::try_from(self.options.logging.log_file.as_ref().unwrap())
                .expect("Failed to create log file spec"),
        );

        logger.start().expect("Failed to initialize logger")
    }

    #[cfg(windows)]
    pub fn is_manager_listen_command(&self) -> bool {
        match &self.options.command {
            DistantSubcommand::Manager(cmd) => cmd.is_listen(),
            _ => false,
        }
    }

    /// Runs the CLI
    pub fn run(self) -> CliResult {
        match self.options.command {
            DistantSubcommand::Client(cmd) => commands::client::run(cmd),
            DistantSubcommand::Generate(cmd) => commands::generate::run(cmd),
            DistantSubcommand::Manager(cmd) => commands::manager::run(cmd),
            DistantSubcommand::Server(cmd) => commands::server::run(cmd),
        }
    }
}
