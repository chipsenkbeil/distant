use std::path::Path;

/// Resolve Windows 8.3 short names (e.g. `RUNNER~1` → `runneradmin`) without
/// resolving symlinks. On non-Windows, this is an identity function.
#[cfg(windows)]
pub fn normalize_path(path: &std::path::Path) -> std::path::PathBuf {
    use std::ffi::OsString;
    use std::os::windows::ffi::{OsStrExt, OsStringExt};

    unsafe extern "system" {
        fn GetLongPathNameW(
            lpszShortPath: *const u16,
            lpszLongPath: *mut u16,
            cchBuffer: u32,
        ) -> u32;
    }

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let len = GetLongPathNameW(wide.as_ptr(), std::ptr::null_mut(), 0);
        if len == 0 {
            return path.to_path_buf();
        }
        let mut buf = vec![0u16; len as usize];
        let len = GetLongPathNameW(wide.as_ptr(), buf.as_mut_ptr(), len);
        if len == 0 {
            return path.to_path_buf();
        }
        buf.truncate(len as usize);
        std::path::PathBuf::from(OsString::from_wide(&buf))
    }
}

#[cfg(not(windows))]
pub fn normalize_path(path: &std::path::Path) -> std::path::PathBuf {
    path.to_path_buf()
}

pub fn ci_path_to_string(path: &Path) -> String {
    // Native Windows OpenSSH expects Windows-style paths, so no conversion needed.
    // Unix conversion was only needed for Cygwin/MSYS2 sshd.
    path.to_string_lossy().to_string()
}

pub mod predicates_ext {
    use std::fmt;

    use predicates::Predicate;
    use predicates::reflection::PredicateReflection;

    /// Checks if lines of text match the provided, trimming each line
    /// of both before comparing.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub struct TrimmedLinesMatchPredicate {
        pattern: String,
    }

    impl TrimmedLinesMatchPredicate {
        pub fn new(pattern: impl Into<String>) -> Self {
            Self {
                pattern: pattern.into(),
            }
        }
    }

    impl fmt::Display for TrimmedLinesMatchPredicate {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "trimmed_lines expects {}", self.pattern)
        }
    }

    impl Predicate<str> for TrimmedLinesMatchPredicate {
        fn eval(&self, variable: &str) -> bool {
            let mut expected = self.pattern.lines();
            let mut actual = variable.lines();

            loop {
                match (expected.next(), actual.next()) {
                    (Some(expected), Some(actual)) => {
                        if expected.trim() != actual.trim() {
                            return false;
                        }
                    }
                    (None, None) => return true,
                    _ => return false,
                }
            }
        }
    }

    impl PredicateReflection for TrimmedLinesMatchPredicate {}
}

pub mod reader {
    use std::io::{BufRead, BufReader, Read};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    use std::{io, thread};

    pub struct ThreadedReader {
        #[allow(dead_code)]
        handle: thread::JoinHandle<io::Result<()>>,
        rx: mpsc::Receiver<String>,
    }

    impl ThreadedReader {
        pub fn new<R>(reader: R) -> Self
        where
            R: Read + Send + 'static,
        {
            let (tx, rx) = mpsc::channel();
            let handle = thread::spawn(move || {
                let mut reader = BufReader::new(reader);
                let mut line = String::new();
                loop {
                    match reader.read_line(&mut line) {
                        Ok(0) => break Ok(()),
                        Ok(_) => {
                            let line2 = line;
                            line = String::new();

                            if let Err(line) = tx.send(line2) {
                                return Err(io::Error::other(format!(
                                    "Failed to pass along line because channel closed! Line: '{}'",
                                    line.0
                                )));
                            }
                        }
                        Err(x) => return Err(x),
                    }
                }
            });
            Self { handle, rx }
        }

        /// Tries to read the next line if available
        pub fn try_read_line(&mut self) -> Option<String> {
            self.rx.try_recv().ok()
        }

        /// Reads the next line, waiting for at minimum "timeout"
        pub fn try_read_line_timeout(&mut self, timeout: Duration) -> Option<String> {
            let start_time = Instant::now();
            let mut checked_at_least_once = false;

            while !checked_at_least_once || start_time.elapsed() < timeout {
                if let Some(line) = self.try_read_line() {
                    return Some(line);
                }

                checked_at_least_once = true;
                thread::sleep(Duration::from_millis(1));
            }

            None
        }

        /// Reads the next line, waiting for at minimum "timeout" before panicking
        pub fn read_line_timeout(&mut self, timeout: Duration) -> String {
            let start_time = Instant::now();
            let mut checked_at_least_once = false;

            while !checked_at_least_once || start_time.elapsed() < timeout {
                if let Some(line) = self.try_read_line() {
                    return line;
                }

                checked_at_least_once = true;
                thread::sleep(Duration::from_millis(1));
            }

            panic!("Reached timeout of {:?}", timeout);
        }

        /// Reads the next line, waiting for at minimum default timeout before panicking
        #[allow(dead_code)]
        pub fn read_line_default_timeout(&mut self) -> String {
            self.read_line_timeout(Self::default_timeout())
        }

        /// Creates a new duration representing a default timeout for the threaded reader
        pub fn default_timeout() -> Duration {
            Duration::from_millis(250)
        }

        /// Waits for reader to shut down, returning the result
        #[allow(dead_code)]
        pub fn wait(self) -> io::Result<()> {
            match self.handle.join() {
                Ok(x) => x,
                Err(x) => std::panic::resume_unwind(x),
            }
        }
    }
}

/// Produces a regex predicate using the given string
pub fn regex_pred(s: &str) -> predicates::str::RegexPredicate {
    use predicates::prelude::*;
    predicate::str::is_match(s).unwrap()
}
