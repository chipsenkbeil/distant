use serde_json::Value;
use std::{
    io::{self, BufRead, BufReader, BufWriter, Write},
    process::Child,
    thread,
    time::Duration,
};
use tokio::sync::mpsc;

const CHANNEL_BUFFER: usize = 100;

pub struct Repl {
    child: Child,
    stdin: mpsc::Sender<String>,
    stdout: mpsc::Receiver<String>,
    stderr: mpsc::Receiver<String>,
    timeout: Option<Duration>,
}

impl Repl {
    /// Create a new [`Repl`] wrapping around a [`Child`]
    pub fn new(mut child: Child, timeout: impl Into<Option<Duration>>) -> Self {
        let mut stdin = BufWriter::new(child.stdin.take().expect("Child missing stdin"));
        let mut stdout = BufReader::new(child.stdout.take().expect("Child missing stdout"));
        let mut stderr = BufReader::new(child.stderr.take().expect("Child missing stderr"));

        let (stdin_tx, mut rx) = mpsc::channel::<String>(CHANNEL_BUFFER);
        thread::spawn(move || {
            while let Some(data) = rx.blocking_recv() {
                if stdin.write_all(data.as_bytes()).is_err() {
                    break;
                }
            }
        });

        let (tx, stdout_rx) = mpsc::channel::<String>(CHANNEL_BUFFER);
        thread::spawn(move || {
            let mut line = String::new();
            while let Ok(n) = stdout.read_line(&mut line) {
                if n == 0 {
                    break;
                }

                if tx.blocking_send(line).is_err() {
                    break;
                }

                line = String::new();
            }
        });

        let (tx, stderr_rx) = mpsc::channel::<String>(CHANNEL_BUFFER);
        thread::spawn(move || {
            let mut line = String::new();
            while let Ok(n) = stderr.read_line(&mut line) {
                if n == 0 {
                    break;
                }

                if tx.blocking_send(line).is_err() {
                    break;
                }

                line = String::new();
            }
        });

        Self {
            child,
            stdin: stdin_tx,
            stdout: stdout_rx,
            stderr: stderr_rx,
            timeout: timeout.into(),
        }
    }

    /// Writes json to the repl over stdin and then waits for json to be received over stdout,
    /// failing if either operation exceeds timeout if set or if the output to stdout is not json,
    /// and returns none if stdout channel has closed
    pub async fn write_and_read_json(
        &mut self,
        value: impl Into<Value>,
    ) -> io::Result<Option<Value>> {
        self.write_json_to_stdin(value).await?;
        self.read_json_from_stdout().await
    }

    /// Writes a line of input to stdin, failing if exceeds timeout if set or if the stdin channel
    /// has been closed. Will append a newline character (`\n`) if line does not end with one.
    pub async fn write_line_to_stdin(&mut self, line: impl Into<String>) -> io::Result<()> {
        let mut line = line.into();
        if !line.ends_with('\n') {
            line.push('\n');
        }

        match self.timeout {
            Some(timeout) => match tokio::time::timeout(timeout, self.stdin.send(line)).await {
                Ok(Ok(_)) => Ok(()),
                Ok(Err(x)) => Err(io::Error::new(io::ErrorKind::BrokenPipe, x)),
                Err(x) => Err(io::Error::new(io::ErrorKind::TimedOut, x)),
            },
            None => self
                .stdin
                .send(line)
                .await
                .map_err(|x| io::Error::new(io::ErrorKind::BrokenPipe, x)),
        }
    }

    /// Writes json value as a line of input to stdin, failing if exceeds timeout if set or if the
    /// stdin channel has been closed. Will append a newline character (`\n`) to JSON string.
    pub async fn write_json_to_stdin(&mut self, value: impl Into<Value>) -> io::Result<()> {
        self.write_line_to_stdin(value.into().to_string()).await
    }

    /// Tries to read a line from stdout, returning none if no stdout is available right now
    ///
    /// Will fail if no more stdout is available
    pub fn try_read_line_from_stdout(&mut self) -> io::Result<Option<String>> {
        match self.stdout.try_recv() {
            Ok(line) => Ok(Some(line)),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => {
                Err(io::Error::from(io::ErrorKind::UnexpectedEof))
            }
        }
    }

    /// Reads a line from stdout, failing if exceeds timeout if set, returning none if the stdout
    /// channel has been closed
    pub async fn read_line_from_stdout(&mut self) -> io::Result<Option<String>> {
        match self.timeout {
            Some(timeout) => match tokio::time::timeout(timeout, self.stdout.recv()).await {
                Ok(x) => Ok(x),
                Err(x) => Err(io::Error::new(io::ErrorKind::TimedOut, x)),
            },
            None => Ok(self.stdout.recv().await),
        }
    }

    /// Reads a line from stdout and parses it as json, failing if unable to parse as json or the
    /// timeout is reached if set, returning none if the stdout channel has been closed
    pub async fn read_json_from_stdout(&mut self) -> io::Result<Option<Value>> {
        match self.read_line_from_stdout().await? {
            Some(line) => {
                let value = serde_json::from_str(&line)
                    .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;
                Ok(Some(value))
            }
            None => Ok(None),
        }
    }

    /// Reads a line from stderr, failing if exceeds timeout if set, returning none if the stderr
    /// channel has been closed
    #[allow(dead_code)]
    pub async fn read_line_from_stderr(&mut self) -> io::Result<Option<String>> {
        match self.timeout {
            Some(timeout) => match tokio::time::timeout(timeout, self.stderr.recv()).await {
                Ok(x) => Ok(x),
                Err(x) => Err(io::Error::new(io::ErrorKind::TimedOut, x)),
            },
            None => Ok(self.stderr.recv().await),
        }
    }

    /// Kills the repl by sending a signal to the process
    pub fn kill(&mut self) -> io::Result<()> {
        self.child.kill()
    }
}

impl Drop for Repl {
    fn drop(&mut self) {
        let _ = self.kill();
    }
}
