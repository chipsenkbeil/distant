use crate::data::PtySize;
use std::{future::Future, pin::Pin, sync::Arc};
use tokio::{io, sync::mpsc};

mod simple;
pub use simple::*;

mod tasks;

/// Alias to the return type of an async function (for use with traits)
pub type FutureReturn<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Exit status of a process
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ExitStatus {
    pub success: bool,
    pub code: Option<i32>,
}

impl ExitStatus {
    /// Produces a new exit status representing a killed process
    pub fn killed() -> Self {
        Self {
            success: false,
            code: None,
        }
    }
}

impl From<std::process::ExitStatus> for ExitStatus {
    fn from(status: std::process::ExitStatus) -> Self {
        Self {
            success: status.success(),
            code: status.code(),
        }
    }
}

/// Represents a notifier for a specific waiting state
#[derive(Debug)]
pub enum WaitNotifier {
    /// Notification has been sent
    Done,

    /// Notification has not been sent
    Pending(mpsc::Sender<ExitStatus>),
}

impl WaitNotifier {
    pub fn is_done(&self) -> bool {
        matches!(self, Self::Done)
    }

    /// Mark wait as completed using provided exit status
    pub fn notify<S>(&mut self, status: S) -> io::Result<()>
    where
        S: Into<ExitStatus>,
    {
        let status = status.into();

        match self {
            Self::Done => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Notifier is closed",
            )),
            Self::Pending(tx) => {
                *self = Self::Done;

                match tx.blocking_send(status) {
                    Ok(_) => Ok(()),
                    Err(x) => Err(io::Error::new(io::ErrorKind::Other, x)),
                }
            }
        }
    }

    /// Mark wait as completed using killed status
    pub fn kill(&mut self) -> io::Result<()> {
        self.notify(ExitStatus::killed())
    }
}

/// Represents the state of waiting for an exit status
#[derive(Debug)]
pub enum Wait {
    /// Exit status is ready
    Ready(ExitStatus),

    /// If receiver for an exit status has been dropped without receiving the status
    Dropped,

    /// Exit status is not ready and has a "oneshot" to be invoked when available
    Pending(mpsc::Receiver<ExitStatus>),
}

impl Wait {
    /// Creates a new channel for when the exit status will be ready
    pub fn new_pending() -> (WaitNotifier, Self) {
        let (tx, rx) = mpsc::channel(1);
        (WaitNotifier::Pending(tx), Wait::Pending(rx))
    }

    pub fn is_pending(&self) -> bool {
        matches!(self, Self::Pending(_))
    }

    /// Converts into an option, returning Some(status) if ready, otherwise None
    ///
    /// Note that this does NOT attempt to resolve a pending instance. To do that,
    /// this requires a mutation and should instead invoke `resolve`.
    pub fn to_option(&self) -> Option<ExitStatus> {
        match self {
            Self::Ready(status) => Some(*status),
            Self::Dropped => None,
            Self::Pending(_) => None,
        }
    }

    /// Waits until the exit status is resolved; can be called repeatedly after being
    /// resolved to immediately return the exit status again
    pub async fn resolve(&mut self) -> io::Result<ExitStatus> {
        match self {
            Self::Ready(status) => Ok(*status),
            Self::Dropped => Err(io::Error::new(
                io::ErrorKind::Other,
                "Internal resolver dropped",
            )),
            Self::Pending(rx) => match rx.recv().await {
                Some(status) => {
                    *self = Self::Ready(status);
                    Ok(status)
                }
                None => {
                    *self = Self::Dropped;
                    Err(io::Error::new(
                        io::ErrorKind::Other,
                        "Internal resolver dropped",
                    ))
                }
            },
        }
    }
}

/// Represents an input channel of a process such as stdin
pub trait InputChannel: Send + Send {
    /// Waits for input to be sent through channel
    fn send<'a>(&'a mut self, data: &[u8]) -> FutureReturn<'a, io::Result<()>>;
}

impl<T: InputChannel + Send + Sync + ?Sized> InputChannel for Arc<T> {
    fn send<'a>(&'a mut self, data: &[u8]) -> FutureReturn<'a, io::Result<()>> {
        InputChannel::send(&mut **self, data)
    }
}

impl InputChannel for mpsc::Sender<Vec<u8>> {
    fn send<'a>(&'a mut self, data: &[u8]) -> FutureReturn<'a, io::Result<()>> {
        let data = data.to_vec();
        Box::pin(async move {
            match mpsc::Sender::send(self, data).await {
                Ok(_) => Ok(()),
                Err(_) => Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "Input channel closed",
                )),
            }
        })
    }
}

/// Represents an output channel of a process such as stdout or stderr
pub trait OutputChannel: Send + Sync {
    /// Waits for next output from channel
    fn recv(&mut self) -> FutureReturn<'_, io::Result<Vec<u8>>>;
}

impl<T: OutputChannel + Send + Sync + ?Sized> OutputChannel for Arc<T> {
    fn recv(&mut self) -> FutureReturn<'_, io::Result<Vec<u8>>> {
        OutputChannel::recv(&mut **self)
    }
}

impl OutputChannel for mpsc::Receiver<Vec<u8>> {
    fn recv(&mut self) -> FutureReturn<'_, io::Result<Vec<u8>>> {
        Box::pin(async move {
            match mpsc::Receiver::recv(self).await {
                Some(data) => Ok(data),
                None => Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "Output channel closed",
                )),
            }
        })
    }
}

/// Represents a process on the remote server
pub trait Process: ProcessStdin + ProcessStdout + ProcessStderr + ProcessKiller {
    /// Represents the id of the process
    fn id(&self) -> usize;

    /// Resize the pty associated with the process
    fn resize_pty(&self, size: PtySize) -> FutureReturn<'_, io::Result<()>>;

    /// Waits for the process to exit, returning the exit status
    ///
    /// If the process has already exited, the status is returned immediately.
    fn wait(&mut self) -> FutureReturn<'_, io::Result<ExitStatus>>;
}

pub trait ProcessStdin: Send {
    /// Writes batch of data to stdin
    fn write_stdin<'a>(&'a mut self, data: &[u8]) -> FutureReturn<'a, io::Result<()>>;

    /// Clones a handle to the stdin channel of the process
    fn clone_stdin(&self) -> Box<dyn ProcessStdin + Send>;

    /// Closes the stdin of the process
    fn close_stdin(&mut self) -> io::Result<()>;
}

pub trait ProcessStdout: Send {
    /// Reads next batch of data from stdout
    fn read_stdout(&mut self) -> FutureReturn<'_, io::Result<Vec<u8>>>;

    /// Clones a handle to the stdout channel of the process
    fn clone_stdout(&self) -> Box<dyn ProcessStdout>;
}

pub trait ProcessStderr: Send {
    /// Reads next batch of data from stderr
    fn read_stderr(&mut self) -> FutureReturn<'_, io::Result<Vec<u8>>>;

    /// Clones a handle to the stderr channel of the process
    fn clone_stderr(&self) -> Box<dyn ProcessStderr>;
}

pub trait ProcessKiller {
    /// Kill the process
    ///
    /// If the process is dead or has already been killed, this will return
    /// an error.
    fn kill(&mut self) -> FutureReturn<'_, io::Result<()>>;

    /// Clone a process killer to support sending signals independently
    fn clone_killer(&self) -> Box<dyn ProcessKiller + Send + Sync>;
}

impl ProcessKiller for mpsc::Sender<()> {
    fn kill(&mut self) -> FutureReturn<'_, io::Result<()>> {
        async fn inner(this: &mut mpsc::Sender<()>) -> io::Result<()> {
            this.send(())
                .await
                .map_err(|x| io::Error::new(io::ErrorKind::BrokenPipe, x))
        }
        Box::pin(inner(self))
    }

    fn clone_killer(&self) -> Box<dyn ProcessKiller + Send + Sync> {
        Box::new(self.clone())
    }
}
