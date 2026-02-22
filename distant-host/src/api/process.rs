use std::future::Future;
use std::pin::Pin;

use distant_core::protocol::{ProcessId, PtySize};
use tokio::io;
use tokio::sync::mpsc;

mod pty;
pub use pty::*;

mod simple;
pub use simple::*;

mod wait;
pub use wait::{ExitStatus, WaitRx};

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
        Err(io::Error::other("Process does not use pty"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    // ---- ProcessKiller for mpsc::Sender<()> ----

    #[test(tokio::test)]
    async fn mpsc_sender_kill_should_succeed_when_receiver_exists() {
        let (tx, mut rx) = mpsc::channel(1);
        let mut killer: Box<dyn ProcessKiller> = Box::new(tx);
        killer.kill().await.unwrap();
        // Verify the () was sent
        assert_eq!(rx.recv().await, Some(()));
    }

    #[test(tokio::test)]
    async fn mpsc_sender_kill_should_fail_when_receiver_dropped() {
        let (tx, rx) = mpsc::channel::<()>(1);
        drop(rx);
        let mut killer: Box<dyn ProcessKiller> = Box::new(tx);
        let result = killer.kill().await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    }

    #[test(tokio::test)]
    async fn mpsc_sender_clone_killer_should_produce_independent_killer() {
        let (tx, mut rx) = mpsc::channel(1);
        let killer: Box<dyn ProcessKiller> = Box::new(tx);
        let mut cloned = killer.clone_killer();
        cloned.kill().await.unwrap();
        assert_eq!(rx.recv().await, Some(()));
    }

    // ---- InputChannel for mpsc::Sender<Vec<u8>> ----

    #[test(tokio::test)]
    async fn mpsc_sender_input_channel_send_should_succeed() {
        let (tx, mut rx) = mpsc::channel::<Vec<u8>>(1);
        let mut input: Box<dyn InputChannel> = Box::new(tx);
        input.send(b"hello").await.unwrap();
        assert_eq!(rx.recv().await, Some(b"hello".to_vec()));
    }

    #[test(tokio::test)]
    async fn mpsc_sender_input_channel_send_should_fail_when_receiver_dropped() {
        let (tx, rx) = mpsc::channel::<Vec<u8>>(1);
        drop(rx);
        let mut input: Box<dyn InputChannel> = Box::new(tx);
        let result = input.send(b"hello").await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    }

    // ---- OutputChannel for mpsc::Receiver<Vec<u8>> ----

    #[test(tokio::test)]
    async fn mpsc_receiver_output_channel_recv_should_return_data() {
        let (tx, rx) = mpsc::channel::<Vec<u8>>(1);
        let mut output: Box<dyn OutputChannel> = Box::new(rx);
        tx.send(b"world".to_vec()).await.unwrap();
        let data = output.recv().await.unwrap();
        assert_eq!(data, Some(b"world".to_vec()));
    }

    #[test(tokio::test)]
    async fn mpsc_receiver_output_channel_recv_should_return_none_when_sender_dropped() {
        let (tx, rx) = mpsc::channel::<Vec<u8>>(1);
        drop(tx);
        let mut output: Box<dyn OutputChannel> = Box::new(rx);
        let data = output.recv().await.unwrap();
        assert_eq!(data, None);
    }

    // ---- NoProcessPty blanket impl ----

    #[test]
    fn no_process_pty_pty_size_should_return_none() {
        let no_pty = NoProcessPtyImpl {};
        assert!(no_pty.pty_size().is_none());
    }

    #[test]
    fn no_process_pty_resize_should_return_error() {
        let no_pty = NoProcessPtyImpl {};
        let size = PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        };
        let result = no_pty.resize_pty(size);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Process does not use pty"));
    }

    #[test]
    fn no_process_pty_clone_pty_should_return_working_clone() {
        let no_pty = NoProcessPtyImpl {};
        let cloned = no_pty.clone_pty();
        // The cloned pty should also return None for size
        assert!(cloned.pty_size().is_none());
        // And error for resize
        let size = PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        };
        assert!(cloned.resize_pty(size).is_err());
        // And the clone of the clone should also work
        let cloned2 = cloned.clone_pty();
        assert!(cloned2.pty_size().is_none());
    }
}
