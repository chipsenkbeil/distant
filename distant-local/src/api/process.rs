use std::future::Future;
use std::pin::Pin;

use tokio::io;
use tokio::sync::mpsc;

use distant_core::protocol::{ProcessId, PtySize};

mod pty;
pub use pty::*;

mod simple;
pub use simple::*;

mod wait;
pub use wait::{ExitStatus, WaitRx, WaitTx};

/// Alias to the return type of an async function (for use with traits)
pub type FutureReturn<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Represents a process on the remote server
pub trait Process: ProcessKiller + ProcessPty {
    /// Represents the id of the process
    fn id(&self) -> ProcessId;

    /// Waits for the process to exit, returning the exit status
    ///
    /// If the process has already exited, the status is returned immediately.
    fn wait(&mut self) -> FutureReturn<'_, io::Result<ExitStatus>>;

    /// Returns a reference to stdin channel if the process still has it associated
    fn stdin(&self) -> Option<&dyn InputChannel>;

    /// Returns a mutable reference to the stdin channel if the process still has it associated
    fn mut_stdin(&mut self) -> Option<&mut (dyn InputChannel + 'static)>;

    /// Takes the stdin channel from the process if it is still associated
    fn take_stdin(&mut self) -> Option<Box<dyn InputChannel>>;

    /// Returns a reference to stdout channel if the process still has it associated
    fn stdout(&self) -> Option<&dyn OutputChannel>;

    /// Returns a mutable reference to the stdout channel if the process still has it associated
    fn mut_stdout(&mut self) -> Option<&mut (dyn OutputChannel + 'static)>;

    /// Takes the stdout channel from the process if it is still associated
    fn take_stdout(&mut self) -> Option<Box<dyn OutputChannel>>;

    /// Returns a reference to stderr channel if the process still has it associated
    fn stderr(&self) -> Option<&dyn OutputChannel>;

    /// Returns a mutable reference to the stderr channel if the process still has it associated
    fn mut_stderr(&mut self) -> Option<&mut (dyn OutputChannel + 'static)>;

    /// Takes the stderr channel from the process if it is still associated
    fn take_stderr(&mut self) -> Option<Box<dyn OutputChannel>>;
}

/// Represents interface that can be used to work with a pty associated with a process
pub trait ProcessPty: Send + Sync {
    /// Returns the current size of the process' pty if it has one
    fn pty_size(&self) -> Option<PtySize>;

    /// Resize the pty associated with the process; returns an error if fails or if the
    /// process does not leverage a pty
    fn resize_pty(&self, size: PtySize) -> io::Result<()>;

    /// Clone a process pty to support reading and updating pty independently
    fn clone_pty(&self) -> Box<dyn ProcessPty>;
}

/// Trait that can be implemented to mark a process as not having a pty
pub trait NoProcessPty: Send + Sync {}

/// Internal type so we can create a dummy instance that implements trait
struct NoProcessPtyImpl {}
impl NoProcessPty for NoProcessPtyImpl {}

impl<T: NoProcessPty> ProcessPty for T {
    fn pty_size(&self) -> Option<PtySize> {
        None
    }

    fn resize_pty(&self, _size: PtySize) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Other,
            "Process does not use pty",
        ))
    }

    fn clone_pty(&self) -> Box<dyn ProcessPty> {
        Box::new(NoProcessPtyImpl {})
    }
}

/// Represents interface that can be used to kill a remote process
pub trait ProcessKiller: Send + Sync {
    /// Kill the process
    ///
    /// If the process is dead or has already been killed, this will return
    /// an error.
    fn kill(&mut self) -> FutureReturn<'_, io::Result<()>>;

    /// Clone a process killer to support sending signals independently
    fn clone_killer(&self) -> Box<dyn ProcessKiller>;
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

    fn clone_killer(&self) -> Box<dyn ProcessKiller> {
        Box::new(self.clone())
    }
}

/// Represents an input channel of a process such as stdin
pub trait InputChannel: Send + Sync {
    /// Sends input through channel, returning unit if succeeds or an error if fails
    fn send<'a>(&'a mut self, data: &[u8]) -> FutureReturn<'a, io::Result<()>>;
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
    /// Waits for next output from channel, returning Some(data) if there is output, None if
    /// the channel has been closed, or bubbles up an error if encountered
    fn recv(&mut self) -> FutureReturn<'_, io::Result<Option<Vec<u8>>>>;
}

impl OutputChannel for mpsc::Receiver<Vec<u8>> {
    fn recv(&mut self) -> FutureReturn<'_, io::Result<Option<Vec<u8>>>> {
        Box::pin(async move { Ok(mpsc::Receiver::recv(self).await) })
    }
}
