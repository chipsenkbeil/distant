mod cli;
mod core;

pub use self::core::{data, net};
use log::error;

/// Represents an error that can be converted into an exit code
pub trait ExitCodeError: std::error::Error {
    fn to_exit_code(&self) -> ExitCode;

    fn to_i32(&self) -> i32 {
        self.to_exit_code() as i32
    }
}

impl ExitCodeError for std::io::Error {
    fn to_exit_code(&self) -> ExitCode {
        use std::io::ErrorKind;
        match self.kind() {
            ErrorKind::ConnectionAborted
            | ErrorKind::ConnectionRefused
            | ErrorKind::ConnectionReset
            | ErrorKind::NotConnected => ExitCode::Unavailable,
            ErrorKind::InvalidData => ExitCode::DataErr,
            ErrorKind::TimedOut => ExitCode::TempFail,
            _ => ExitCode::IoError,
        }
    }
}

impl ExitCodeError for core::net::TransportError {
    fn to_exit_code(&self) -> ExitCode {
        match self {
            core::net::TransportError::IoError(x) => x.to_exit_code(),
            _ => ExitCode::Protocol,
        }
    }
}

impl<T: ExitCodeError + 'static> From<T> for Box<dyn ExitCodeError> {
    fn from(x: T) -> Self {
        Box::new(x)
    }
}

/// Exit codes following https://www.freebsd.org/cgi/man.cgi?query=sysexits&sektion=3
#[derive(Copy, Clone, PartialEq, Eq, Hash)]
pub enum ExitCode {
    /// EX_USAGE (64) - being used when arguments missing or bad arguments provided to CLI
    Usage = 64,

    /// EX_DATAERR (65) - being used when bad data received not in UTF-8 format or transport data
    /// is bad
    DataErr = 65,

    /// EX_NOINPUT (66) - being used when not getting expected data from launch
    NoInput = 66,

    /// EX_NOHOST (68) - being used when failed to resolve a host
    NoHost = 68,

    /// EX_UNAVAILABLE (69) - being used when IO error encountered where connection is problem
    Unavailable = 69,

    /// EX_OSERR (71) - being used when fork failed
    OsErr = 71,

    /// EX_IOERR (74) - being used as catchall for IO errors
    IoError = 74,

    /// EX_TEMPFAIL (75) - being used when we get a timeout
    TempFail = 75,

    /// EX_PROTOCOL (76) - being used as catchall for transport errors
    Protocol = 76,
}

/// Main entrypoint into the program
pub fn run() {
    let opt = cli::opt::Opt::load();
    init_logging(&opt.common);
    if let Err(x) = opt.subcommand.run(opt.common) {
        error!("{}", x);
        std::process::exit(x.to_i32());
    }
}

fn init_logging(opt: &cli::opt::CommonOpt) {
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
    let logger = Logger::with(builder.build()).format_for_files(flexi_logger::opt_format);

    // If provided, log to file instead of stderr
    let logger = if let Some(path) = opt.log_file.as_ref() {
        logger.log_to_file(FileSpec::try_from(path).expect("Failed to create log file spec"))
    } else {
        logger
    };

    logger.start().expect("Failed to initialize logger");
}
