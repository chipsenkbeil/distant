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
            .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;
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
        let mut child = pty_slave
            .spawn_command(cmd)
            .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;

        // NOTE: Need to drop slave to close out file handles and avoid deadlock when waiting on
        //       the child
        drop(pty_slave);

        // Spawn a blocking task to process submitting stdin async
        let (stdin_tx, mut stdin_rx) = mpsc::channel::<Vec<u8>>(1);
        let mut stdin_writer = pty_master
            .take_writer()
            .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;
        let stdin_task = tokio::task::spawn_blocking(move || {
            while let Some(input) = stdin_rx.blocking_recv() {
                if stdin_writer.write_all(&input).is_err() {
                    break;
                }
            }
        });

        // Spawn a blocking task to process receiving stdout async
        let (stdout_tx, stdout_rx) = mpsc::channel::<Vec<u8>>(1);
        let mut stdout_reader = pty_master
            .try_clone_reader()
            .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;
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
                .map_err(|x| io::Error::new(io::ErrorKind::Other, x))
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
