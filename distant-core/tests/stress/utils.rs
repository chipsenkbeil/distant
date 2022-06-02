use std::path::PathBuf;

/// Initializes logging (should only call once)
pub fn init_logging(path: impl Into<PathBuf>) -> flexi_logger::LoggerHandle {
    use flexi_logger::{FileSpec, LevelFilter, LogSpecification, Logger};
    let modules = &["distant", "distant_core", "distant_ssh2"];

    // Disable logging for everything but our binary, which is based on verbosity
    let mut builder = LogSpecification::builder();
    builder.default(LevelFilter::Off);

    // For each module, configure logging
    for module in modules {
        builder.module(module, LevelFilter::Trace);
    }

    // Create our logger, but don't initialize yet
    let logger = Logger::with(builder.build())
        .format_for_files(flexi_logger::opt_format)
        .log_to_file(FileSpec::try_from(path).expect("Failed to create log file spec"));

    logger.start().expect("Failed to initialize logger")
}
