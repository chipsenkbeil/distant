mod data;
mod net;
mod opt;
mod subcommand;
mod utils;

use log::error;
pub use opt::Opt;
use std::path::PathBuf;

lazy_static::lazy_static! {
    static ref PROJECT_DIRS: directories::ProjectDirs =
        directories::ProjectDirs::from(
            "org",
            "senkbeil",
            "distant",
        ).expect("Failed to find valid home directory path");
    static ref SESSION_PATH: PathBuf = PROJECT_DIRS.cache_dir().join("session");
}

/// Main entrypoint into the program
pub fn run() {
    let opt = Opt::load();
    init_logging(&opt.common);
    if let Err(x) = opt.subcommand.run(opt.common) {
        error!("{}", x);
    }
}

pub fn init_logging(opt: &opt::CommonOpt) {
    use flexi_logger::{FileSpec, LevelFilter, LogSpecification, Logger};
    let module = "distant";

    // Disable logging for everything but our binary, which is based on verbosity
    let mut builder = LogSpecification::builder();
    builder.default(LevelFilter::Off).module(
        module,
        match opt.verbose {
            0 => LevelFilter::Warn,
            1 => LevelFilter::Info,
            2 => LevelFilter::Debug,
            _ => LevelFilter::Trace,
        },
    );

    // If quiet, we suppress all output
    if opt.quiet {
        builder.module(module, LevelFilter::Off);
    }

    // Create our logger, but don't initialize yet
    let logger = Logger::with(builder.build());

    // If provided, log to file instead of stderr
    let logger = if let Some(path) = opt.log_file.as_ref() {
        logger.log_to_file(FileSpec::try_from(path).expect("Failed to create log file spec"))
    } else {
        logger
    };

    logger.start().expect("Failed to initialize logger");
}
