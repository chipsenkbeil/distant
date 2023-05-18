use std::future::Future;
use std::io;
use std::path::PathBuf;

use distant_net::server::Reply;
use log::*;
use tokio::task::JoinHandle;

use crate::api::local::process::{
    InputChannel, OutputChannel, Process, ProcessKiller, ProcessPty, PtyProcess, SimpleProcess,
};
use crate::protocol::{Environment, ProcessId, PtySize, Response};

/// Holds information related to a spawned process on the server
pub struct ProcessInstance {
    pub cmd: String,
    pub args: Vec<String>,

    pub id: ProcessId,
    pub stdin: Option<Box<dyn InputChannel>>,
    pub killer: Box<dyn ProcessKiller>,
    pub pty: Box<dyn ProcessPty>,

    stdout_task: Option<JoinHandle<io::Result<()>>>,
    stderr_task: Option<JoinHandle<io::Result<()>>>,
    wait_task: Option<JoinHandle<io::Result<()>>>,
}

impl Drop for ProcessInstance {
    /// Closes stdin and attempts to kill the process when dropped
    fn drop(&mut self) {
        // Drop stdin first to close it
        self.stdin = None;

        // Clear out our tasks if we still have them
        let stdout_task = self.stdout_task.take();
        let stderr_task = self.stderr_task.take();
        let wait_task = self.wait_task.take();

        // Attempt to kill the process, which is an async operation that we
        // will spawn a task to handle
        let id = self.id;
        let mut killer = self.killer.clone_killer();
        tokio::spawn(async move {
            if let Err(x) = killer.kill().await {
                error!("Failed to kill process {} when dropped: {}", id, x);

                if let Some(task) = stdout_task.as_ref() {
                    task.abort();
                }
                if let Some(task) = stderr_task.as_ref() {
                    task.abort();
                }
                if let Some(task) = wait_task.as_ref() {
                    task.abort();
                }
            }
        });
    }
}

impl ProcessInstance {
    pub fn spawn(
        cmd: String,
        environment: Environment,
        current_dir: Option<PathBuf>,
        pty: Option<PtySize>,
        reply: Box<dyn Reply<Data = Response>>,
    ) -> io::Result<Self> {
        // Build out the command and args from our string
        let mut cmd_and_args = if cfg!(windows) {
            winsplit::split(&cmd)
        } else {
            shell_words::split(&cmd).map_err(|x| io::Error::new(io::ErrorKind::InvalidInput, x))?
        };

        if cmd_and_args.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Command was empty",
            ));
        }

        // Split command from arguments, where arguments could be empty
        let args = cmd_and_args.split_off(1);
        let cmd = cmd_and_args.into_iter().next().unwrap();

        let mut child: Box<dyn Process> = match pty {
            Some(size) => Box::new(PtyProcess::spawn(
                cmd.clone(),
                args.clone(),
                environment,
                current_dir,
                size,
            )?),
            None => Box::new(SimpleProcess::spawn(
                cmd.clone(),
                args.clone(),
                environment,
                current_dir,
            )?),
        };

        let id = child.id();
        let stdin = child.take_stdin();
        let stdout = child.take_stdout();
        let stderr = child.take_stderr();
        let killer = child.clone_killer();
        let pty = child.clone_pty();

        // Spawn a task that sends stdout as a response
        let stdout_task = match stdout {
            Some(stdout) => {
                let reply = reply.clone_reply();
                let task = tokio::spawn(stdout_task(id, stdout, reply));
                Some(task)
            }
            None => None,
        };

        // Spawn a task that sends stderr as a response
        let stderr_task = match stderr {
            Some(stderr) => {
                let reply = reply.clone_reply();
                let task = tokio::spawn(stderr_task(id, stderr, reply));
                Some(task)
            }
            None => None,
        };

        // Spawn a task that waits on the process to exit but can also
        // kill the process when triggered
        let wait_task = Some(tokio::spawn(wait_task(id, child, reply)));

        Ok(ProcessInstance {
            cmd,
            args,
            id,
            stdin,
            killer,
            pty,
            stdout_task,
            stderr_task,
            wait_task,
        })
    }

    /// Invokes the function once the process has completed
    ///
    /// NOTE: Can only be used with one function. All future calls
    ///       will do nothing
    pub fn on_done<F, R>(&mut self, f: F)
    where
        F: FnOnce(io::Result<()>) -> R + Send + 'static,
        R: Future<Output = ()> + Send,
    {
        if let Some(task) = self.wait_task.take() {
            tokio::spawn(async move {
                f(task
                    .await
                    .unwrap_or_else(|x| Err(io::Error::new(io::ErrorKind::Other, x))))
                .await
            });
        }
    }
}

async fn stdout_task(
    id: ProcessId,
    mut stdout: Box<dyn OutputChannel>,
    reply: Box<dyn Reply<Data = Response>>,
) -> io::Result<()> {
    loop {
        match stdout.recv().await {
            Ok(Some(data)) => {
                reply.send(Response::ProcStdout { id, data }).await?;
            }
            Ok(None) => return Ok(()),
            Err(x) => return Err(x),
        }
    }
}

async fn stderr_task(
    id: ProcessId,
    mut stderr: Box<dyn OutputChannel>,
    reply: Box<dyn Reply<Data = Response>>,
) -> io::Result<()> {
    loop {
        match stderr.recv().await {
            Ok(Some(data)) => {
                reply.send(Response::ProcStderr { id, data }).await?;
            }
            Ok(None) => return Ok(()),
            Err(x) => return Err(x),
        }
    }
}

async fn wait_task(
    id: ProcessId,
    mut child: Box<dyn Process>,
    reply: Box<dyn Reply<Data = Response>>,
) -> io::Result<()> {
    let status = child.wait().await;

    match status {
        Ok(status) => {
            reply
                .send(Response::ProcDone {
                    id,
                    success: status.success,
                    code: status.code,
                })
                .await
        }
        Err(x) => reply.send(Response::from(x)).await,
    }
}
