use super::{
    wait, ExitStatus, FutureReturn, InputChannel, NoProcessPty, OutputChannel, Process,
    ProcessKiller, WaitRx,
};
use std::{ffi::OsStr, process::Stdio};
use tokio::{io, process::Command, sync::mpsc, task::JoinHandle};

mod tasks;

/// Represents a simple process that does not have a pty
pub struct SimpleProcess {
    id: usize,
    stdin: Option<Box<dyn InputChannel>>,
    stdout: Option<Box<dyn OutputChannel>>,
    stderr: Option<Box<dyn OutputChannel>>,
    stdin_task: Option<JoinHandle<io::Result<()>>>,
    stdout_task: Option<JoinHandle<io::Result<()>>>,
    stderr_task: Option<JoinHandle<io::Result<()>>>,
    kill_tx: mpsc::Sender<()>,
    wait: WaitRx,
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

        let (kill_tx, mut kill_rx) = mpsc::channel(1);
        let (mut wait_tx, wait_rx) = wait::channel();

        tokio::spawn(async move {
            tokio::select! {
                _ = kill_rx.recv() => {
                    let status = match child.kill().await {
                        Ok(_) => ExitStatus::killed(),
                        Err(x) => ExitStatus::from(x),
                    };

                    // TODO: Keep track of io error
                    let _ = wait_tx.send(status).await;
                }
                status = child.wait() => {
                    // TODO: Keep track of io error
                    let _ = wait_tx.send(status).await;
                }
            }
        });

        Ok(Self {
            id: rand::random(),
            stdin: Some(Box::new(stdin_ch)),
            stdout: Some(Box::new(stdout_ch)),
            stderr: Some(Box::new(stderr_ch)),
            stdin_task: Some(stdin_task),
            stdout_task: Some(stdout_task),
            stderr_task: Some(stderr_task),
            kill_tx,
            wait: wait_rx,
        })
    }
}

impl Process for SimpleProcess {
    fn id(&self) -> usize {
        self.id
    }

    fn wait(&mut self) -> FutureReturn<'_, io::Result<ExitStatus>> {
        async fn inner(this: &mut SimpleProcess) -> io::Result<ExitStatus> {
            let mut status = this.wait.recv().await?;

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

    fn stdin(&self) -> Option<&dyn InputChannel> {
        self.stdin.as_deref()
    }

    fn mut_stdin(&mut self) -> Option<&mut (dyn InputChannel + 'static)> {
        self.stdin.as_deref_mut()
    }

    fn take_stdin(&mut self) -> Option<Box<dyn InputChannel>> {
        self.stdin.take()
    }

    fn stdout(&self) -> Option<&dyn OutputChannel> {
        self.stdout.as_deref()
    }

    fn mut_stdout(&mut self) -> Option<&mut (dyn OutputChannel + 'static)> {
        self.stdout.as_deref_mut()
    }

    fn take_stdout(&mut self) -> Option<Box<dyn OutputChannel>> {
        self.stdout.take()
    }

    fn stderr(&self) -> Option<&dyn OutputChannel> {
        self.stderr.as_deref()
    }

    fn mut_stderr(&mut self) -> Option<&mut (dyn OutputChannel + 'static)> {
        self.stderr.as_deref_mut()
    }

    fn take_stderr(&mut self) -> Option<Box<dyn OutputChannel>> {
        self.stderr.take()
    }
}

impl NoProcessPty for SimpleProcess {}

impl ProcessKiller for SimpleProcess {
    fn kill(&mut self) -> FutureReturn<'_, io::Result<()>> {
        async fn inner(this: &mut SimpleProcess) -> io::Result<()> {
            this.kill_tx
                .send(())
                .await
                .map_err(|x| io::Error::new(io::ErrorKind::BrokenPipe, x))
        }
        Box::pin(inner(self))
    }

    fn clone_killer(&self) -> Box<dyn ProcessKiller> {
        Box::new(self.kill_tx.clone())
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

    fn clone_killer(&self) -> Box<dyn ProcessKiller> {
        Box::new(self.clone())
    }
}
