use crate::cli::fixtures::DistantServerCtx;
use predicates::prelude::*;
use std::{
    env, io,
    path::PathBuf,
    process::{Command, Stdio},
    sync::mpsc,
    time::{Duration, Instant},
};

lazy_static::lazy_static! {
    /// Predicate that checks for a single line that is a failure
    pub static ref FAILURE_LINE: predicates::str::RegexPredicate =
        regex_pred(r"^Failed \(.*\): '.*'\.\n$");
}

/// Produces a regex predicate using the given string
pub fn regex_pred(s: &str) -> predicates::str::RegexPredicate {
    predicate::str::is_match(s).unwrap()
}

/// Creates a random tenant name
pub fn random_tenant() -> String {
    format!("test-tenant-{}", rand::random::<u16>())
}

/// Initializes logging (should only call once)
pub fn init_logging(path: impl Into<PathBuf>) -> flexi_logger::LoggerHandle {
    use flexi_logger::{FileSpec, LevelFilter, LogSpecification, Logger};
    let modules = &["distant", "distant_core"];

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

pub fn friendly_recv_line(
    receiver: &mpsc::Receiver<String>,
    duration: Duration,
) -> io::Result<String> {
    let start = Instant::now();
    loop {
        if let Ok(line) = receiver.try_recv() {
            break Ok(line);
        }

        if start.elapsed() > duration {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("Failed to receive line after {}s", duration.as_secs_f32()),
            ));
        }

        std::thread::yield_now();
    }
}

pub fn spawn_line_reader<T>(mut reader: T) -> mpsc::Receiver<String>
where
    T: std::io::Read + Send + 'static,
{
    let id = rand::random::<u8>();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = String::new();
        let mut tmp = [0; 1024];
        while let Ok(n) = reader.read(&mut tmp) {
            if n == 0 {
                break;
            }

            let data = String::from_utf8_lossy(&tmp[..n]);
            buf.push_str(data.as_ref());

            // Send all complete lines
            match buf.rfind('\n') {
                Some(idx) => {
                    let remaining = buf.split_off(idx + 1);
                    for line in buf.lines() {
                        tx.send(line.to_string()).unwrap();
                    }
                    buf = remaining;
                }
                None => {}
            }
        }

        // If something is remaining at end, also send it
        if !buf.is_empty() {
            tx.send(buf).unwrap();
        }
    });

    rx
}

/// Produces a new command for distant using the given subcommand
pub fn distant_subcommand(ctx: &DistantServerCtx, subcommand: &str) -> Command {
    let mut cmd = Command::new(cargo_bin(env!("CARGO_PKG_NAME")));
    cmd.arg(subcommand)
        .args(&["--session", "environment"])
        .env("DISTANT_HOST", ctx.addr.ip().to_string())
        .env("DISTANT_PORT", ctx.addr.port().to_string())
        .env("DISTANT_AUTH_KEY", ctx.auth_key.as_str())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

/// Look up the path to a cargo-built binary within an integration test
///
/// Taken from https://github.com/assert-rs/assert_cmd/blob/036ef47b8ad170dcaf4eaf4412c0b48fd5b6ef6e/src/cargo.rs#L199
fn cargo_bin<S: AsRef<str>>(name: S) -> PathBuf {
    cargo_bin_str(name.as_ref())
}

fn cargo_bin_str(name: &str) -> PathBuf {
    let env_var = format!("CARGO_BIN_EXE_{}", name);
    std::env::var_os(&env_var)
        .map(|p| p.into())
        .unwrap_or_else(|| target_dir().join(format!("{}{}", name, env::consts::EXE_SUFFIX)))
}

// Adapted from
// https://github.com/rust-lang/cargo/blob/485670b3983b52289a2f353d589c57fae2f60f82/tests/testsuite/support/mod.rs#L507
fn target_dir() -> PathBuf {
    env::current_exe()
        .ok()
        .map(|mut path| {
            path.pop();
            if path.ends_with("deps") {
                path.pop();
            }
            path
        })
        .unwrap()
}
