mod buf;
mod constants;
mod environment;
mod exit;
mod link;
mod msg;
mod opt;
mod output;
mod session;
mod stdin;
mod subcommand;
mod utils;

use log::error;

pub use exit::{ExitCode, ExitCodeError};

/// Main entrypoint into the program
pub fn run() {
    let opt = opt::Opt::load();
    let logger = init_logging(&opt.common, opt.subcommand.is_remote_process());
    if let Err(x) = opt.subcommand.run(opt.common) {
        if !x.is_silent() {
            error!("Exiting due to error: {}", x);
        }
        logger.flush();
        logger.shutdown();

        std::process::exit(x.to_i32());
    }
}

fn init_logging(opt: &opt::CommonOpt, is_remote_process: bool) -> flexi_logger::LoggerHandle {
    use flexi_logger::{FileSpec, LevelFilter, LogSpecification, Logger};
    let modules = &["distant", "distant_core"];

    // Disable logging for everything but our binary, which is based on verbosity
    let mut builder = LogSpecification::builder();
    builder.default(LevelFilter::Off);

    // For each module, configure logging
    for module in modules {
        builder.module(module, opt.log_level.to_log_level_filter());

        // If quiet, we suppress all logging output
        //
        // NOTE: For a process request, unless logging to a file, we also suppress logging output
        //       to avoid unexpected results when being treated like a process
        //
        //       Without this, CI tests can sporadically fail when getting the exit code of a
        //       process because an error log is provided about failing to broadcast a response
        //       on the client side
        if opt.quiet || (is_remote_process && opt.log_file.is_none()) {
            builder.module(module, LevelFilter::Off);
        }
    }

    // Create our logger, but don't initialize yet
    let logger = Logger::with(builder.build()).format_for_files(flexi_logger::opt_format);

    // If provided, log to file instead of stderr
    let logger = if let Some(path) = opt.log_file.as_ref() {
        logger.log_to_file(FileSpec::try_from(path).expect("Failed to create log file spec"))
    } else {
        logger
    };

    logger.start().expect("Failed to initialize logger")
}
