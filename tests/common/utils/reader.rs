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
                        // Consume the line and create an empty line to
                        // be filled in next time
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
