use super::{
    tasks, ExitStatus, FutureReturn, InputChannel, OutputChannel, Process, ProcessKiller,
    ProcessStderr, ProcessStdin, ProcessStdout, Wait,
};
use crate::{
    constants::{MAX_PIPE_CHUNK_SIZE, READ_PAUSE_MILLIS},
    data::PtySize,
};
use derive_more::{Display, Error};
use portable_pty::{native_pty_system, CommandBuilder, PtySize as PortablePtySize, PtySystem};
use std::{
    ffi::OsStr,
    future::Future,
    pin::Pin,
    process::Stdio,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::{
    io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    process::{Child, ChildStderr, ChildStdin, ChildStdout, Command},
    sync::{mpsc, oneshot, Mutex},
    task::{JoinError, JoinHandle},
};

pub struct SimpleProcess {
    id: usize,
    child: Child,
    stdin: Arc<Mutex<Box<dyn InputChannel>>>,
    stdout: Arc<Mutex<Box<dyn OutputChannel>>>,
    stderr: Arc<Mutex<Box<dyn OutputChannel>>>,
    exit_status: Wait,
}

impl SimpleProcess {
    /// Spawns a new simple process
    pub fn spawn<S, I, S2>(program: S, args: I) -> io::Result<Self>
    where
        S: AsRef<OsStr>,
        I: IntoIterator<Item = S2>,
        S2: AsRef<OsStr>,
    {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take().unwrap();
        let (stdout_task, stdout_ch) = tasks::spawn_read_task(stdout, 1);

        let stderr = child.stderr.take().unwrap();
        let (stderr_task, stderr_ch) = tasks::spawn_read_task(stderr, 1);

        let stdin = child.stdin.take().unwrap();
        let (stdin_task, stdin_ch) = tasks::spawn_write_task(stdin, 1);

        let (wait_tx, wait) = Wait::new_pending();
        let (kill_tx, kill_rx) = oneshot::channel();
        let wait_task = tokio::spawn(wait_handler(
            child,
            kill_rx,
            stdin_task,
            stdout_task,
            stderr_task,
        ));
    }
}

impl Process for SimpleProcess {
    fn id(&self) -> usize {
        self.id
    }

    /// Resize the pty associated with the process
    fn resize_pty(&self, _size: PtySize) -> FutureReturn<'_, io::Result<()>> {
        Box::pin(async {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Process is not within a pty",
            ))
        })
    }

    fn wait(&mut self) -> FutureReturn<'_, io::Result<ExitStatus>> {
        Box::pin(self.exit_status.resolve())
    }
}

impl ProcessStdin for SimpleProcess {
    fn write_stdin<'a>(&'a mut self, data: &[u8]) -> FutureReturn<'a, io::Result<()>> {
        async fn inner(this: &mut SimpleProcess, data: &[u8]) -> io::Result<()> {
            this.stdin.lock().await.send(data).await
        }
        Box::pin(inner(self, data))
    }

    fn clone_stdin(&self) -> Box<dyn InputChannel + Send> {
        self.stdin.clone()
    }
}

impl ProcessStdout for SimpleProcess {
    fn read_stdout(&mut self) -> FutureReturn<'_, io::Result<Vec<u8>>>;

    fn clone_stdout(&self) -> Box<dyn OutputChannel + Send>;
}

impl ProcessStderr for SimpleProcess {
    fn read_stderr(&mut self) -> FutureReturn<'_, io::Result<Vec<u8>>>;

    fn clone_stderr(&self) -> Box<dyn OutputChannel + Send>;
}

impl ProcessKiller for SimpleProcess {
    /// Kill the process
    ///
    /// If the process is dead or has already been killed, this will return
    /// an error.
    fn kill(&mut self) -> FutureReturn<'_, io::Result<()>>;

    /// Clone a process killer to support sending signals independently
    fn clone_killer(&self) -> Box<dyn ProcessKiller + Send + Sync>;
}
