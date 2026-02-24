use std::ffi::OsStr;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, Weak};

use distant_core::protocol::Environment;
use log::*;
use portable_pty::{CommandBuilder, MasterPty, PtySize as PortablePtySize};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use super::{
    wait, ExitStatus, FutureReturn, InputChannel, OutputChannel, Process, ProcessId, ProcessKiller,
    ProcessPty, PtySize, WaitRx,
};
use crate::constants::{MAX_PIPE_CHUNK_SIZE, READ_PAUSE_DURATION};

/// Represents a process that is associated with a pty
pub struct PtyProcess {
    id: ProcessId,
    pty_master: Option<Arc<Mutex<Box<dyn MasterPty + Send>>>>,
    stdin: Option<Box<dyn InputChannel>>,
    stdout: Option<Box<dyn OutputChannel>>,
    stdin_task: Option<JoinHandle<()>>,
    stdout_task: Option<JoinHandle<io::Result<()>>>,
    kill_tx: mpsc::Sender<()>,
    wait: WaitRx,
}

impl PtyProcess {
    /// Spawns a new simple process
    pub fn spawn<S, I, S2>(
        program: S,
        args: I,
        environment: Environment,
        current_dir: Option<PathBuf>,
        size: PtySize,
    ) -> io::Result<Self>
    where
        S: AsRef<OsStr>,
        I: IntoIterator<Item = S2>,
        S2: AsRef<OsStr>,
    {
        let id = rand::random();

        // Establish our new pty for the given size
        let pty_system = portable_pty::native_pty_system();
        let pty_pair = pty_system
            .openpty(PortablePtySize {
                rows: size.rows,
                cols: size.cols,
                pixel_width: size.pixel_width,
                pixel_height: size.pixel_height,
            })
            .map_err(io::Error::other)?;
        let pty_master = pty_pair.master;
        let pty_slave = pty_pair.slave;

        // Spawn our process within the pty
        let mut cmd = CommandBuilder::new(program);
        cmd.args(args);
        if let Some(path) = current_dir {
            cmd.cwd(path);
        }
        for (key, value) in environment {
            cmd.env(key, value);
        }
        let mut child = pty_slave.spawn_command(cmd).map_err(io::Error::other)?;

        // NOTE: Need to drop slave to close out file handles and avoid deadlock when waiting on
        //       the child
        drop(pty_slave);

        // Spawn a blocking task to process submitting stdin async
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(1);
        let mut stdin_writer = pty_master.take_writer().map_err(io::Error::other)?;
        let stdin_task = tokio::task::spawn_blocking(move || {
            while let Some(input) = stdin_rx.blocking_recv() {
                if stdin_writer.write_all(&input).is_err() {
                    break;
                }
            }
        });

        // Spawn a blocking task to process receiving stdout async
        let (stdout_tx, stdout_rx) = mpsc::channel::<Vec<u8>>(1);
        let mut stdout_reader = pty_master.try_clone_reader().map_err(io::Error::other)?;
        let stdout_task = tokio::task::spawn_blocking(move || {
            let mut buf: [u8; MAX_PIPE_CHUNK_SIZE] = [0; MAX_PIPE_CHUNK_SIZE];
            loop {
                match stdout_reader.read(&mut buf) {
                    Ok(n) if n > 0 => {
                        stdout_tx.blocking_send(buf[..n].to_vec()).map_err(|_| {
                            io::Error::new(io::ErrorKind::BrokenPipe, "Output channel closed")
                        })?;
                    }
                    Ok(_) => return Ok(()),
                    Err(x) => return Err(x),
                }
            }
        });

        let (kill_tx, mut kill_rx) = mpsc::channel(1);
        let (mut wait_tx, wait_rx) = wait::channel();

        tokio::spawn(async move {
            loop {
                match (child.try_wait(), kill_rx.try_recv()) {
                    (Ok(Some(status)), _) => {
                        trace!(
                            "Pty process {id} has exited: success = {}",
                            status.success()
                        );

                        if let Err(x) = wait_tx
                            .send(ExitStatus {
                                success: status.success(),
                                code: None,
                            })
                            .await
                        {
                            error!("Pty process {id} exit status lost: {x}");
                        }

                        break;
                    }
                    (_, Ok(_)) => {
                        trace!("Pty process {id} received kill request");

                        if let Err(x) = wait_tx.kill().await {
                            error!("Pty process {id} exit status lost: {x}");
                        }

                        break;
                    }
                    (Err(x), _) => {
                        trace!("Pty process {id} failed to wait");

                        if let Err(x) = wait_tx.send(x).await {
                            error!("Pty process {id} exit status lost: {x}");
                        }

                        break;
                    }
                    _ => {
                        tokio::time::sleep(READ_PAUSE_DURATION).await;
                        continue;
                    }
                }
            }
        });

        Ok(Self {
            id,
            pty_master: Some(Arc::new(Mutex::new(pty_master))),
            stdin: Some(Box::new(stdin_tx)),
            stdout: Some(Box::new(stdout_rx)),
            stdin_task: Some(stdin_task),
            stdout_task: Some(stdout_task),
            kill_tx,
            wait: wait_rx,
        })
    }

    /// Return a weak reference to the pty master
    fn pty_master(&self) -> Weak<Mutex<Box<dyn MasterPty + Send>>> {
        self.pty_master
            .as_ref()
            .map(Arc::downgrade)
            .unwrap_or_default()
    }
}

impl Process for PtyProcess {
    fn id(&self) -> ProcessId {
        self.id
    }

    fn wait(&mut self) -> FutureReturn<'_, io::Result<ExitStatus>> {
        async fn inner(this: &mut PtyProcess) -> io::Result<ExitStatus> {
            let mut status = this.wait.recv().await?;

            // Drop our master once we have finished
            let _ = this.pty_master.take();

            if let Some(task) = this.stdin_task.take() {
                task.abort();
            }
            if let Some(task) = this.stdout_task.take() {
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
        None
    }

    fn mut_stderr(&mut self) -> Option<&mut (dyn OutputChannel + 'static)> {
        None
    }

    fn take_stderr(&mut self) -> Option<Box<dyn OutputChannel>> {
        None
    }
}

impl ProcessKiller for PtyProcess {
    fn kill(&mut self) -> FutureReturn<'_, io::Result<()>> {
        async fn inner(this: &mut PtyProcess) -> io::Result<()> {
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
pub struct PtyProcessKiller(mpsc::Sender<()>);

impl ProcessKiller for PtyProcessKiller {
    fn kill(&mut self) -> FutureReturn<'_, io::Result<()>> {
        async fn inner(this: &mut PtyProcessKiller) -> io::Result<()> {
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

impl ProcessPty for PtyProcess {
    fn pty_size(&self) -> Option<PtySize> {
        PtyProcessMaster(self.pty_master()).pty_size()
    }

    fn resize_pty(&self, size: PtySize) -> io::Result<()> {
        PtyProcessMaster(self.pty_master()).resize_pty(size)
    }

    fn clone_pty(&self) -> Box<dyn ProcessPty> {
        PtyProcessMaster(self.pty_master()).clone_pty()
    }
}

#[derive(Clone)]
struct PtyProcessMaster(Weak<Mutex<Box<dyn MasterPty + Send>>>);

impl ProcessPty for PtyProcessMaster {
    fn pty_size(&self) -> Option<PtySize> {
        if let Some(master) = Weak::upgrade(&self.0) {
            master.lock().unwrap().get_size().ok().map(|size| PtySize {
                rows: size.rows,
                cols: size.cols,
                pixel_width: size.pixel_width,
                pixel_height: size.pixel_height,
            })
        } else {
            None
        }
    }

    fn resize_pty(&self, size: PtySize) -> io::Result<()> {
        if let Some(master) = Weak::upgrade(&self.0) {
            master
                .lock()
                .unwrap()
                .resize(PortablePtySize {
                    rows: size.rows,
                    cols: size.cols,
                    pixel_width: size.pixel_width,
                    pixel_height: size.pixel_height,
                })
                .map_err(io::Error::other)
        } else {
            Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Pty master has been dropped",
            ))
        }
    }

    fn clone_pty(&self) -> Box<dyn ProcessPty> {
        Box::new(self.clone())
    }
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    //! Tests for `PtyProcess` covering spawn, process trait accessors, wait/exit,
    //! kill via clone, PTY resize/clone, and the `PtyProcessMaster` weak-reference machinery.

    use super::*;
    use distant_core::protocol::Environment;

    fn empty_env() -> Environment {
        Environment::new()
    }

    fn default_size() -> PtySize {
        PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        }
    }

    mod spawn {
        use super::*;

        #[test_log::test(tokio::test)]
        async fn with_valid_program_succeeds() {
            let proc = PtyProcess::spawn("echo", ["hello"], empty_env(), None, default_size());
            assert!(proc.is_ok());
        }

        #[test_log::test(tokio::test)]
        async fn with_invalid_program_returns_error() {
            let result = PtyProcess::spawn(
                "nonexistent_program_that_does_not_exist_xyz",
                Vec::<String>::new(),
                empty_env(),
                None,
                default_size(),
            );
            assert!(result.is_err());
        }

        #[test_log::test(tokio::test)]
        async fn with_current_dir_succeeds() {
            let dir = tempfile::tempdir().unwrap();
            let proc = PtyProcess::spawn(
                "pwd",
                Vec::<String>::new(),
                empty_env(),
                Some(dir.path().to_path_buf()),
                default_size(),
            );
            assert!(proc.is_ok());
        }

        #[test_log::test(tokio::test)]
        async fn with_environment_variable() {
            let mut env = Environment::new();
            env.insert("MY_TEST_VAR".to_string(), "my_test_value".to_string());
            let mut proc =
                PtyProcess::spawn("env", Vec::<String>::new(), env, None, default_size()).unwrap();

            // Read stdout and verify the env var is actually visible to the spawned process
            let mut stdout = proc.take_stdout().unwrap();
            let mut all_output = Vec::new();
            while let Ok(Some(data)) = stdout.recv().await {
                all_output.extend_from_slice(&data);
            }
            let output = String::from_utf8_lossy(&all_output);
            assert!(
                output.contains("MY_TEST_VAR=my_test_value"),
                "Expected env output to contain MY_TEST_VAR=my_test_value, got: {output}"
            );
        }
    }

    mod process_trait {
        use super::*;

        #[test_log::test(tokio::test)]
        async fn id_returns_a_value() {
            let proc =
                PtyProcess::spawn("echo", ["test"], empty_env(), None, default_size()).unwrap();
            let _id: ProcessId = proc.id();
        }

        #[test_log::test(tokio::test)]
        async fn stdin_is_some_initially() {
            let proc =
                PtyProcess::spawn("echo", ["test"], empty_env(), None, default_size()).unwrap();
            assert!(proc.stdin().is_some());
        }

        #[test_log::test(tokio::test)]
        async fn stdout_is_some_initially() {
            let proc =
                PtyProcess::spawn("echo", ["test"], empty_env(), None, default_size()).unwrap();
            assert!(proc.stdout().is_some());
        }

        #[test_log::test(tokio::test)]
        async fn stderr_is_always_none() {
            let proc =
                PtyProcess::spawn("echo", ["test"], empty_env(), None, default_size()).unwrap();
            assert!(proc.stderr().is_none());
        }

        #[test_log::test(tokio::test)]
        async fn take_stdin_removes_it() {
            let mut proc =
                PtyProcess::spawn("echo", ["test"], empty_env(), None, default_size()).unwrap();
            let stdin = proc.take_stdin();
            assert!(stdin.is_some());
            assert!(proc.stdin().is_none());
            assert!(proc.take_stdin().is_none());
        }

        #[test_log::test(tokio::test)]
        async fn take_stdout_removes_it() {
            let mut proc =
                PtyProcess::spawn("echo", ["test"], empty_env(), None, default_size()).unwrap();
            let stdout = proc.take_stdout();
            assert!(stdout.is_some());
            assert!(proc.stdout().is_none());
            assert!(proc.take_stdout().is_none());
        }

        #[test_log::test(tokio::test)]
        async fn take_stderr_is_always_none() {
            let mut proc =
                PtyProcess::spawn("echo", ["test"], empty_env(), None, default_size()).unwrap();
            assert!(proc.take_stderr().is_none());
        }

        #[test_log::test(tokio::test)]
        async fn mut_stdin_is_some_initially() {
            let mut proc =
                PtyProcess::spawn("echo", ["test"], empty_env(), None, default_size()).unwrap();
            assert!(proc.mut_stdin().is_some());
        }

        #[test_log::test(tokio::test)]
        async fn mut_stdout_is_some_initially() {
            let mut proc =
                PtyProcess::spawn("echo", ["test"], empty_env(), None, default_size()).unwrap();
            assert!(proc.mut_stdout().is_some());
        }

        #[test_log::test(tokio::test)]
        async fn mut_stderr_is_always_none() {
            let mut proc =
                PtyProcess::spawn("echo", ["test"], empty_env(), None, default_size()).unwrap();
            assert!(proc.mut_stderr().is_none());
        }
    }

    mod wait_and_exit {
        use super::*;

        #[test_log::test(tokio::test)]
        async fn echo_exits_successfully_with_code_zero() {
            let mut proc =
                PtyProcess::spawn("echo", ["hello"], empty_env(), None, default_size()).unwrap();
            let status = proc.wait().await.unwrap();
            assert!(status.success);
            assert_eq!(status.code, Some(0));
        }

        #[test_log::test(tokio::test)]
        async fn false_command_exits_with_nonzero_status() {
            let mut proc = PtyProcess::spawn(
                "false",
                Vec::<String>::new(),
                empty_env(),
                None,
                default_size(),
            )
            .unwrap();
            let status = proc.wait().await.unwrap();
            assert!(!status.success);
        }

        #[test_log::test(tokio::test)]
        async fn kill_then_wait_returns_killed_status() {
            let mut proc =
                PtyProcess::spawn("sleep", ["60"], empty_env(), None, default_size()).unwrap();

            ProcessKiller::kill(&mut proc).await.unwrap();
            let status = proc.wait().await.unwrap();
            assert!(!status.success);
        }

        #[test_log::test(tokio::test)]
        async fn wait_drops_pty_master() {
            let mut proc =
                PtyProcess::spawn("echo", ["done"], empty_env(), None, default_size()).unwrap();

            // Before wait, pty_size should return Some
            assert!(proc.pty_size().is_some());

            let _status = proc.wait().await.unwrap();

            // After wait, the pty_master is taken, so pty_size returns None
            assert!(proc.pty_size().is_none());
        }
    }

    mod clone_killer {
        use super::*;

        #[test_log::test(tokio::test)]
        async fn cloned_killer_can_kill_process() {
            let mut proc =
                PtyProcess::spawn("sleep", ["60"], empty_env(), None, default_size()).unwrap();
            let mut killer = proc.clone_killer();
            killer.kill().await.unwrap();

            let status = proc.wait().await.unwrap();
            assert!(!status.success);
        }

        #[test_log::test(tokio::test)]
        async fn clone_killer_returns_independent_killer() {
            let proc =
                PtyProcess::spawn("sleep", ["60"], empty_env(), None, default_size()).unwrap();

            let killer1 = proc.clone_killer();
            let _killer2 = killer1.clone_killer();
        }
    }

    mod process_pty {
        use super::*;

        #[test_log::test(tokio::test)]
        async fn pty_size_returns_some() {
            let proc =
                PtyProcess::spawn("echo", ["test"], empty_env(), None, default_size()).unwrap();

            let size = proc.pty_size();
            assert!(size.is_some());
            let size = size.unwrap();
            assert_eq!(size.rows, 24);
            assert_eq!(size.cols, 80);
        }

        #[test_log::test(tokio::test)]
        async fn resize_pty_succeeds() {
            let proc =
                PtyProcess::spawn("echo", ["test"], empty_env(), None, default_size()).unwrap();

            let new_size = PtySize {
                rows: 40,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            };
            let result = proc.resize_pty(new_size);
            assert!(result.is_ok());

            // Verify the resize took effect
            let size = proc.pty_size().unwrap();
            assert_eq!(size.rows, 40);
            assert_eq!(size.cols, 120);
        }

        #[test_log::test(tokio::test)]
        async fn clone_pty_returns_working_clone() {
            let proc =
                PtyProcess::spawn("echo", ["test"], empty_env(), None, default_size()).unwrap();

            let pty_clone = proc.clone_pty();
            let size = pty_clone.pty_size();
            assert!(size.is_some());
            assert_eq!(size.unwrap().rows, 24);
            assert_eq!(size.unwrap().cols, 80);
        }

        #[test_log::test(tokio::test)]
        async fn clone_pty_resize_succeeds() {
            let proc =
                PtyProcess::spawn("echo", ["test"], empty_env(), None, default_size()).unwrap();

            let pty_clone = proc.clone_pty();
            let new_size = PtySize {
                rows: 50,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            };
            assert!(pty_clone.resize_pty(new_size).is_ok());
        }

        #[test_log::test(tokio::test)]
        async fn pty_master_dropped_returns_none_for_size() {
            // Create a PtyProcessMaster with a dead weak reference
            let master = PtyProcessMaster(Weak::new());
            assert!(master.pty_size().is_none());
        }

        #[test_log::test(tokio::test)]
        async fn pty_master_dropped_returns_error_for_resize() {
            let master = PtyProcessMaster(Weak::new());
            let size = PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            };
            let result = master.resize_pty(size);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
        }

        #[test_log::test(tokio::test)]
        async fn pty_master_dropped_clone_also_returns_none() {
            let master = PtyProcessMaster(Weak::new());
            let cloned = master.clone_pty();
            assert!(cloned.pty_size().is_none());
        }

        #[test_log::test(tokio::test)]
        async fn pty_master_method_returns_weak_ref() {
            let proc =
                PtyProcess::spawn("echo", ["test"], empty_env(), None, default_size()).unwrap();

            // pty_master() should return a weak ref that can be upgraded
            let weak = proc.pty_master();
            assert!(weak.upgrade().is_some());
        }

        #[test_log::test(tokio::test)]
        async fn pty_master_method_with_no_master_returns_default_weak() {
            let mut proc =
                PtyProcess::spawn("echo", ["done"], empty_env(), None, default_size()).unwrap();

            // Wait so that pty_master is taken
            let _status = proc.wait().await.unwrap();

            let weak = proc.pty_master();
            assert!(weak.upgrade().is_none());
        }
    }
}
