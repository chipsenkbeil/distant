use super::{
    tasks, ExitStatus, FutureReturn, InputChannel, OutputChannel, Process, ProcessKiller,
    ProcessStderr, ProcessStdin, ProcessStdout, Wait,
};
use crate::data::PtySize;
use std::{ffi::OsStr, process::Stdio, sync::Arc};
use tokio::{
    io,
    process::Command,
    sync::{mpsc, Mutex},
    task::JoinHandle,
};

pub struct SimpleProcess {
    id: usize,
    stdin: SimpleProcessStdin,
    stdout: SimpleProcessStdout,
    stderr: SimpleProcessStderr,
    stdin_task: Option<JoinHandle<io::Result<()>>>,
    stdout_task: Option<JoinHandle<io::Result<()>>>,
    stderr_task: Option<JoinHandle<io::Result<()>>>,
    kill_tx: mpsc::Sender<()>,
    wait: Wait,
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

        let (kill_tx, kill_rx) = mpsc::channel(1);
        let (notifier, wait) = Wait::new_pending();

        tokio::spawn(async move {
            tokio::select! {
                _ = kill_rx.recv() => {
                    if child.kill().await.is_ok() {
                        // TODO: Keep track of io error
                        let _ = notifier.notify(ExitStatus::killed());
                    }
                }
                exit_status = child.wait() => {
                    // TODO: Keep track of io error
                    if let Ok(status) = exit_status {
                        let _ = notifier.notify(status);
                    }
                }
            }
        });

        Ok(Self {
            id: rand::random(),
            stdin: SimpleProcessStdin(Arc::new(Mutex::new(stdin_ch))),
            stdout: SimpleProcessStdout(Arc::new(Mutex::new(stdout_ch))),
            stderr: SimpleProcessStderr(Arc::new(Mutex::new(stderr_ch))),
            stdin_task: Some(stdin_task),
            stdout_task: Some(stdout_task),
            stderr_task: Some(stderr_task),
            kill_tx,
            wait,
        })
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
        // Box::pin(self.exit_status.resolve())
        async fn inner(this: &mut SimpleProcess) -> io::Result<ExitStatus> {
            let mut status = this.wait.resolve().await?;

            if let Some(task) = this.stdin_task.take() {
                task.abort();
            }
            if let Some(task) = this.stdout_task.take() {
                let _ = task.await;
            }
            if let Some(task) = this.stderr_task.take() {
                let _ = task.await;
            }

            if status.success && status.code.is_none() {
                status.code = Some(0);
            }
            Ok(status)
        }
        Box::pin(inner(self))
    }
}

impl ProcessStdin for SimpleProcess {
    fn write_stdin<'a>(&'a mut self, data: &[u8]) -> FutureReturn<'a, io::Result<()>> {
        self.stdin.write_stdin(data)
    }

    fn clone_stdin(&self) -> Box<dyn InputChannel + Send> {
        Box::new(self.stdin.clone())
    }
}

impl ProcessStdout for SimpleProcess {
    fn read_stdout(&mut self) -> FutureReturn<'_, io::Result<Vec<u8>>> {
        self.stdout.read_stdout()
    }

    fn clone_stdout(&self) -> Box<dyn OutputChannel + Send> {
        Box::new(self.stdout.clone())
    }
}

impl ProcessStderr for SimpleProcess {
    fn read_stderr(&mut self) -> FutureReturn<'_, io::Result<Vec<u8>>> {
        self.stderr.read_stderr()
    }

    fn clone_stderr(&self) -> Box<dyn OutputChannel + Send> {
        Box::new(self.stderr.clone())
    }
}

impl ProcessKiller for SimpleProcess {
    /// Kill the process
    ///
    /// If the process is dead or has already been killed, this will return
    /// an error.
    fn kill(&mut self) -> FutureReturn<'_, io::Result<()>> {
        async fn inner(this: &mut SimpleProcess) -> io::Result<()> {
            this.kill_tx
                .send(())
                .await
                .map_err(|x| io::Error::new(io::ErrorKind::BrokenPipe, x))
        }
        Box::pin(inner(self))
    }

    /// Clone a process killer to support sending signals independently
    fn clone_killer(&self) -> Box<dyn ProcessKiller + Send + Sync> {
        Box::new(self.kill_tx.clone())
    }
}

#[derive(Clone)]
pub struct SimpleProcessStdin(Arc<Mutex<Box<dyn InputChannel>>>);

impl InputChannel for SimpleProcessStdin {
    fn send<'a>(&'a mut self, data: &[u8]) -> FutureReturn<'a, io::Result<()>> {
        async fn inner(this: &mut SimpleProcessStdin, data: &[u8]) -> io::Result<()> {
            this.0.lock().await.send(data).await
        }

        // TODO: CHIP CHIP CHIP -- Do not clone data! Figure out why there are lifetime problems!
        let data2 = data.to_vec();

        Box::pin(inner(self, &data2))
    }
}

impl ProcessStdin for SimpleProcessStdin {
    fn write_stdin<'a>(&'a mut self, data: &[u8]) -> FutureReturn<'a, io::Result<()>> {
        InputChannel::send(self, data)
    }

    fn clone_stdin(&self) -> Box<dyn InputChannel + Send> {
        Box::new(self.clone())
    }
}

#[derive(Clone)]
pub struct SimpleProcessStdout(Arc<Mutex<Box<dyn OutputChannel>>>);

impl OutputChannel for SimpleProcessStdout {
    fn recv(&mut self) -> FutureReturn<'_, io::Result<Vec<u8>>> {
        async fn inner(this: &mut SimpleProcessStdout) -> io::Result<Vec<u8>> {
            this.0.lock().await.recv().await
        }
        Box::pin(inner(self))
    }
}

impl ProcessStdout for SimpleProcessStdout {
    fn read_stdout(&mut self) -> FutureReturn<'_, io::Result<Vec<u8>>> {
        OutputChannel::recv(self)
    }

    fn clone_stdout(&self) -> Box<dyn OutputChannel + Send> {
        Box::new(self.clone())
    }
}

#[derive(Clone)]
pub struct SimpleProcessStderr(Arc<Mutex<Box<dyn OutputChannel>>>);

impl OutputChannel for SimpleProcessStderr {
    fn recv(&mut self) -> FutureReturn<'_, io::Result<Vec<u8>>> {
        async fn inner(this: &mut SimpleProcessStderr) -> io::Result<Vec<u8>> {
            this.0.lock().await.recv().await
        }
        Box::pin(inner(self))
    }
}

impl ProcessStderr for SimpleProcessStderr {
    fn read_stderr(&mut self) -> FutureReturn<'_, io::Result<Vec<u8>>> {
        async fn inner(this: &mut SimpleProcessStderr) -> io::Result<Vec<u8>> {
            this.0.lock().await.recv().await
        }
        Box::pin(inner(self))
    }

    fn clone_stderr(&self) -> Box<dyn OutputChannel + Send> {
        Box::new(self.clone())
    }
}

#[derive(Clone)]
pub struct SimpleProcessKiller(mpsc::Sender<()>);

impl ProcessKiller for SimpleProcessKiller {
    fn kill(&mut self) -> FutureReturn<'_, io::Result<()>> {
        async fn inner(this: &mut SimpleProcessKiller) -> io::Result<()> {
            this.0
                .send(())
                .await
                .map_err(|x| io::Error::new(io::ErrorKind::BrokenPipe, x))
        }
        Box::pin(inner(self))
    }

    fn clone_killer(&self) -> Box<dyn ProcessKiller + Send + Sync> {
        Box::new(self.clone())
    }
}
