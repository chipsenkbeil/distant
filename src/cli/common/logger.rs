use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

use log::{LevelFilter, Log, Metadata, Record};

/// A file-based logger implementing [`log::Log`].
///
/// Filters log messages by module prefix and level, writing matching records to a file.
/// The format replicates `flexi_logger::opt_format`:
/// ```text
/// [2016-01-13 15:25:01.640870 +01:00] INFO [src/foo/bar:26] Task message
/// ```
pub struct FileLogger {
    modules: Vec<String>,
    level: LevelFilter,
    file: Mutex<File>,
}

impl FileLogger {
    /// Initializes the global logger.
    ///
    /// Writes to `path`, accepting records from `modules` at or above `level`.
    /// Panics if the logger cannot be set (i.e. a logger is already installed).
    pub fn init(modules: Vec<String>, level: LevelFilter, path: &Path) {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap_or_else(|e| panic!("Failed to open log file {path:?}: {e}"));

        let logger = Box::new(Self {
            modules,
            level,
            file: Mutex::new(file),
        });

        log::set_boxed_logger(logger).expect("Failed to set logger");
        log::set_max_level(level);
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

impl Log for FileLogger {
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
        if let Ok(mut f) = self.file.lock() {
            let _ = writeln!(f, "{line}");
        }
    }

    fn flush(&self) {
        if let Ok(mut f) = self.file.lock() {
            let _ = f.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use log::Level;
    use test_log::test;

    use super::*;

    /// Helper: create a FileLogger without installing it as the global logger
    /// (since tests run in parallel and only one global logger can exist).
    fn make_logger(modules: Vec<String>, level: LevelFilter, path: &Path) -> FileLogger {
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();

        FileLogger {
            modules,
            level,
            file: Mutex::new(file),
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
