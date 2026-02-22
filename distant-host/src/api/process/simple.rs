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

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;

    use distant_core::protocol::Environment;

    fn empty_env() -> Environment {
        Environment::new()
    }

    mod spawn {
        use super::*;

        #[test_log::test(tokio::test)]
        async fn with_valid_program_succeeds() {
            let proc = SimpleProcess::spawn("echo", ["hello"], empty_env(), None);
            assert!(proc.is_ok());
        }

        #[test_log::test(tokio::test)]
        async fn with_invalid_program_returns_error() {
            let result = SimpleProcess::spawn(
                "nonexistent_program_that_does_not_exist_xyz",
                Vec::<String>::new(),
                empty_env(),
                None,
            );
            assert!(result.is_err());
        }
    }

    mod process_trait {
        use super::*;

        #[test_log::test(tokio::test)]
        async fn id_returns_nonzero_value() {
            // id is randomly generated, so it could theoretically be 0,
            // but we test it's at least set
            let proc = SimpleProcess::spawn("echo", ["test"], empty_env(), None).unwrap();
            // ProcessId is u32, just verify it's accessible
            let _id: ProcessId = proc.id();
        }

        #[test_log::test(tokio::test)]
        async fn stdin_is_some_initially() {
            let proc = SimpleProcess::spawn("echo", ["test"], empty_env(), None).unwrap();
            assert!(proc.stdin().is_some());
        }

        #[test_log::test(tokio::test)]
        async fn stdout_is_some_initially() {
            let proc = SimpleProcess::spawn("echo", ["test"], empty_env(), None).unwrap();
            assert!(proc.stdout().is_some());
        }

        #[test_log::test(tokio::test)]
        async fn stderr_is_some_initially() {
            let proc = SimpleProcess::spawn("echo", ["test"], empty_env(), None).unwrap();
            assert!(proc.stderr().is_some());
        }

        #[test_log::test(tokio::test)]
        async fn take_stdin_removes_it() {
            let mut proc = SimpleProcess::spawn("echo", ["test"], empty_env(), None).unwrap();
            let stdin = proc.take_stdin();
            assert!(stdin.is_some());
            assert!(proc.stdin().is_none());
            assert!(proc.take_stdin().is_none());
        }

        #[test_log::test(tokio::test)]
        async fn take_stdout_removes_it() {
            let mut proc = SimpleProcess::spawn("echo", ["test"], empty_env(), None).unwrap();
            let stdout = proc.take_stdout();
            assert!(stdout.is_some());
            assert!(proc.stdout().is_none());
            assert!(proc.take_stdout().is_none());
        }

        #[test_log::test(tokio::test)]
        async fn take_stderr_removes_it() {
            let mut proc = SimpleProcess::spawn("echo", ["test"], empty_env(), None).unwrap();
            let stderr = proc.take_stderr();
            assert!(stderr.is_some());
            assert!(proc.stderr().is_none());
            assert!(proc.take_stderr().is_none());
        }

        #[test_log::test(tokio::test)]
        async fn mut_stdin_is_some_initially() {
            let mut proc = SimpleProcess::spawn("echo", ["test"], empty_env(), None).unwrap();
            assert!(proc.mut_stdin().is_some());
        }

        #[test_log::test(tokio::test)]
        async fn mut_stdout_is_some_initially() {
            let mut proc = SimpleProcess::spawn("echo", ["test"], empty_env(), None).unwrap();
            assert!(proc.mut_stdout().is_some());
        }

        #[test_log::test(tokio::test)]
        async fn mut_stderr_is_some_initially() {
            let mut proc = SimpleProcess::spawn("echo", ["test"], empty_env(), None).unwrap();
            assert!(proc.mut_stderr().is_some());
        }
    }

    mod wait_and_exit {
        use super::*;

        #[test_log::test(tokio::test)]
        async fn echo_exits_successfully_with_code_zero() {
            let mut proc = SimpleProcess::spawn("echo", ["hello"], empty_env(), None).unwrap();
            let status = proc.wait().await.unwrap();
            assert!(status.success);
            assert_eq!(status.code, Some(0));
        }

        #[test_log::test(tokio::test)]
        async fn false_command_exits_with_nonzero_code() {
            let mut proc =
                SimpleProcess::spawn("false", Vec::<String>::new(), empty_env(), None).unwrap();
            let status = proc.wait().await.unwrap();
            assert!(!status.success);
            assert!(status.code.is_some());
            assert_ne!(status.code.unwrap(), 0);
        }

        #[test_log::test(tokio::test)]
        async fn kill_then_wait_returns_killed_status() {
            // Spawn a long-running process
            let mut proc = SimpleProcess::spawn("sleep", ["60"], empty_env(), None).unwrap();

            ProcessKiller::kill(&mut proc).await.unwrap();
            let status = proc.wait().await.unwrap();
            assert!(!status.success);
        }
    }

    mod stdout_capture {
        use super::*;

        #[test_log::test(tokio::test)]
        async fn captures_stdout_from_echo() {
            let mut proc = SimpleProcess::spawn("echo", ["hello"], empty_env(), None).unwrap();
            let mut stdout = proc.take_stdout().unwrap();

            // Read from stdout
            let data = stdout.recv().await.unwrap();
            assert!(data.is_some());
            let bytes = data.unwrap();
            let output = String::from_utf8_lossy(&bytes);
            assert!(output.contains("hello"));

            // Wait for process to finish
            let status = proc.wait().await.unwrap();
            assert!(status.success);
        }
    }

    mod clone_killer {
        use super::*;

        #[test_log::test(tokio::test)]
        async fn cloned_killer_can_kill_process() {
            let mut proc = SimpleProcess::spawn("sleep", ["60"], empty_env(), None).unwrap();
            let mut killer = proc.clone_killer();
            killer.kill().await.unwrap();

            let status = proc.wait().await.unwrap();
            assert!(!status.success);
        }
    }

    mod no_process_pty {
        use super::*;
        use crate::api::process::ProcessPty;

        #[test_log::test(tokio::test)]
        async fn pty_size_returns_none() {
            let proc = SimpleProcess::spawn("echo", ["test"], empty_env(), None).unwrap();
            assert!(proc.pty_size().is_none());
        }

        #[test_log::test(tokio::test)]
        async fn resize_pty_returns_error() {
            let proc = SimpleProcess::spawn("echo", ["test"], empty_env(), None).unwrap();
            let size = distant_core::protocol::PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            };
            let result = proc.resize_pty(size);
            assert!(result.is_err());
        }
    }

    mod spawn_with_current_dir {
        use super::*;

        #[test_log::test(tokio::test)]
        async fn uses_specified_current_dir() {
            let dir = tempfile::tempdir().unwrap();
            let mut proc = SimpleProcess::spawn(
                "pwd",
                Vec::<String>::new(),
                empty_env(),
                Some(dir.path().to_path_buf()),
            )
            .unwrap();

            let mut stdout = proc.take_stdout().unwrap();
            let data = stdout.recv().await.unwrap();
            assert!(data.is_some());
            let bytes = data.unwrap();
            let output = String::from_utf8_lossy(&bytes);

            // The output should contain the directory name
            // Note: canonicalize might add /private on macOS
            let canon = dir.path().canonicalize().unwrap();
            assert!(
                output.trim() == canon.to_str().unwrap()
                    || output.trim() == dir.path().to_str().unwrap(),
                "Expected pwd output '{}' to match '{}' or '{}'",
                output.trim(),
                canon.display(),
                dir.path().display(),
            );

            let status = proc.wait().await.unwrap();
            assert!(status.success);
        }
    }
}
