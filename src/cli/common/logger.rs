use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use log::{LevelFilter, Log, Metadata, Record};

/// A logger implementing [`log::Log`] that writes to one or more outputs.
///
/// Filters log messages by module prefix and level, writing matching records to every
/// configured output (file, stderr, or both). The format replicates `flexi_logger::opt_format`:
/// ```text
/// [2016-01-13 15:25:01.640870 +01:00] INFO [src/foo/bar:26] Task message
/// ```
///
/// Construct via [`Logger::builder`] to configure outputs before installing.
pub struct Logger {
    modules: Vec<String>,
    level: LevelFilter,
    outputs: Vec<Mutex<Box<dyn Write + Send>>>,
}

/// Builder for configuring and installing a [`Logger`].
///
/// Supports file output, stderr output, or both. At least one output must be
/// configured before calling [`init`](LoggerBuilder::init).
pub struct LoggerBuilder {
    modules: Vec<String>,
    level: LevelFilter,
    file_path: Option<PathBuf>,
    stderr: bool,
}

impl Logger {
    /// Returns a new [`LoggerBuilder`] with default settings.
    ///
    /// The defaults are: no modules, [`LevelFilter::Info`], no outputs.
    pub fn builder() -> LoggerBuilder {
        LoggerBuilder {
            modules: Vec::new(),
            level: LevelFilter::Info,
            file_path: None,
            stderr: false,
        }
    }

    /// Returns `true` if the record's module path starts with one of our allowed prefixes.
    fn matches_module(&self, record: &Record) -> bool {
        let target = record.module_path().unwrap_or(record.target());
        self.modules.iter().any(|m| target.starts_with(m))
    }

    /// Formats a record in the `flexi_logger::opt_format` style.
    fn format(record: &Record) -> String {
        use std::fmt::Write;

        let now = chrono::Local::now();
        let mut buf = String::with_capacity(128);

        let _ = write!(
            buf,
            "[{}] {} ",
            now.format("%Y-%m-%d %H:%M:%S%.6f %:z"),
            record.level(),
        );

        if let (Some(file), Some(line)) = (record.file(), record.line()) {
            let _ = write!(buf, "[{file}:{line}] ");
        }

        let _ = write!(buf, "{}", record.args());
        buf
    }
}

impl LoggerBuilder {
    /// Sets the module prefixes that this logger will accept records from.
    pub fn modules(mut self, modules: Vec<String>) -> Self {
        self.modules = modules;
        self
    }

    /// Sets the maximum log level.
    pub fn level(mut self, level: LevelFilter) -> Self {
        self.level = level;
        self
    }

    /// Configures the logger to append to a file at `path`.
    pub fn file(mut self, path: &Path) -> Self {
        self.file_path = Some(path.to_path_buf());
        self
    }

    /// Configures the logger to also write to stderr.
    #[allow(dead_code)]
    pub fn stderr(mut self) -> Self {
        self.stderr = true;
        self
    }

    /// Installs the logger as the global logger.
    ///
    /// # Errors
    ///
    /// Returns an error if no outputs could be opened or the global logger
    /// is already set. If the file fails to open but stderr is enabled,
    /// logs the failure to stderr and continues stderr-only.
    pub fn init(self) -> Result<(), Box<dyn std::error::Error>> {
        let mut outputs: Vec<Mutex<Box<dyn Write + Send>>> = Vec::new();

        if let Some(ref path) = self.file_path {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match OpenOptions::new().create(true).append(true).open(path) {
                Ok(f) => outputs.push(Mutex::new(Box::new(f))),
                Err(e) => {
                    if self.stderr {
                        eprintln!(
                            "warning: failed to open log file {path:?}: {e}, \
                             logging to stderr only"
                        );
                    } else {
                        return Err(format!("Failed to open log file {path:?}: {e}").into());
                    }
                }
            }
        }

        if self.stderr {
            outputs.push(Mutex::new(Box::new(std::io::stderr())));
        }

        if outputs.is_empty() {
            return Err("no log outputs configured".into());
        }

        let logger = Box::new(Logger {
            modules: self.modules,
            level: self.level,
            outputs,
        });

        log::set_boxed_logger(logger).map_err(|e| format!("Failed to set logger: {e}"))?;
        log::set_max_level(self.level);
        Ok(())
    }
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        if !self.matches_module(record) {
            return;
        }

        let line = Self::format(record);
        for output in &self.outputs {
            if let Ok(mut w) = output.lock() {
                let _ = writeln!(w, "{line}");
            }
        }
    }

    fn flush(&self) {
        for output in &self.outputs {
            if let Ok(mut w) = output.lock() {
                let _ = w.flush();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Read;

    use log::Level;
    use test_log::test;

    use super::*;

    /// Helper: create a Logger without installing it as the global logger
    /// (since tests run in parallel and only one global logger can exist).
    fn make_logger(modules: Vec<String>, level: LevelFilter, path: &Path) -> Logger {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();

        Logger {
            modules,
            level,
            outputs: vec![Mutex::new(Box::new(file))],
        }
    }

    /// Helper macro to create a Record and log it, avoiding lifetime issues with format_args!
    macro_rules! log_to {
        ($logger:expr, $level:expr, $target:expr, $msg:expr) => {{
            $logger.log(
                &Record::builder()
                    .level($level)
                    .target($target)
                    .module_path(Some($target))
                    .file(Some("src/test.rs"))
                    .line(Some(42))
                    .args(format_args!("{}", $msg))
                    .build(),
            );
        }};
    }

    #[test]
    fn file_logger_filters_by_module() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.path().join("test.log");

        let logger = make_logger(vec!["distant".to_string()], LevelFilter::Trace, &path);

        // Should be logged (module starts with "distant")
        log_to!(logger, Level::Info, "distant::core", "good message");

        // Should NOT be logged (module doesn't match)
        log_to!(logger, Level::Info, "tokio::runtime", "bad message");

        logger.flush();

        let mut contents = String::new();
        File::open(&path)
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();
        assert!(contents.contains("good message"));
        assert!(!contents.contains("bad message"));
    }

    #[test]
    fn file_logger_filters_by_level() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.path().join("test.log");

        let logger = make_logger(vec!["distant".to_string()], LevelFilter::Warn, &path);

        // Should be logged (Warn >= Warn)
        log_to!(logger, Level::Warn, "distant", "warn msg");

        // Should NOT be logged (Info < Warn)
        log_to!(logger, Level::Info, "distant", "info msg");

        logger.flush();

        let mut contents = String::new();
        File::open(&path)
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();
        assert!(contents.contains("warn msg"));
        assert!(!contents.contains("info msg"));
    }

    #[test]
    fn file_logger_format_matches_opt_format() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.path().join("test.log");

        let logger = make_logger(vec!["distant".to_string()], LevelFilter::Trace, &path);

        log_to!(logger, Level::Info, "distant", "task completed");
        logger.flush();

        let mut contents = String::new();
        File::open(&path)
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();

        // Format: [YYYY-MM-DD HH:MM:SS.ffffff +HH:MM] LEVEL [file:line] message
        assert!(
            contents.contains("] INFO [src/test.rs:42] task completed"),
            "got: {contents}"
        );

        // Verify timestamp bracket
        assert!(contents.starts_with('['), "got: {contents}");
    }

    #[test]
    fn file_logger_flush_writes_data() {
        let dir = assert_fs::TempDir::new().unwrap();
        let path = dir.path().join("test.log");

        let logger = make_logger(vec!["distant".to_string()], LevelFilter::Trace, &path);

        log_to!(logger, Level::Debug, "distant", "flush test");
        logger.flush();

        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(contents.contains("flush test"));
    }
}
