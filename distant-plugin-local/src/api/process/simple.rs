use std::ffi::OsStr;
use std::path::PathBuf;
use std::process::Stdio;

use distant_core::protocol::Environment;
use log::*;
use tokio::io;
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::{
    wait, ExitStatus, FutureReturn, InputChannel, NoProcessPty, OutputChannel, Process, ProcessId,
    ProcessKiller, WaitRx,
};

mod tasks;

/// Represents a simple process that does not have a pty
pub struct SimpleProcess {
    id: ProcessId,
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
    pub fn spawn<S, I, S2>(
        program: S,
        args: I,
        environment: Environment,
        current_dir: Option<PathBuf>,
    ) -> io::Result<Self>
    where
        S: AsRef<OsStr>,
        I: IntoIterator<Item = S2>,
        S2: AsRef<OsStr>,
    {
        let id = rand::random();
        let mut child = {
            let mut command = Command::new(program);

            if let Some(path) = current_dir {
                command.current_dir(path);
            }

            command
                .envs(environment)
                .args(args)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()?
        };

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
                    trace!("Pty process {id} received kill request");
                    let status = match child.kill().await {
                        Ok(_) => ExitStatus::killed(),
                        Err(x) => ExitStatus::from(x),
                    };

                    trace!(
                        "Simple process {id} has exited: success = {}, code = {}",
                        status.success,
                        status.code.map(|code| code.to_string())
                            .unwrap_or_else(|| "<terminated>".to_string()),
                    );

                    if let Err(x) = wait_tx.send(status).await {
                        error!("Simple process {id} exit status lost: {x}");
                    }
                }
                status = child.wait() => {
                    match &status {
                        Ok(status) => trace!(
                            "Simple process {id} has exited: success = {}, code = {}",
                            status.success(),
                            status.code()
                                .map(|code| code.to_string())
                                .unwrap_or_else(|| "<terminated>".to_string()),
                        ),
                        Err(_) => trace!("Simple process {id} failed to wait"),
                    }

                    if let Err(x) = wait_tx.send(status).await {
                        error!("Simple process {id} exit status lost: {x}");
                    }
                }
            }
        });

        Ok(Self {
            id,
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
    fn id(&self) -> ProcessId {
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
