use crate::{
    api::local::process::{
        InputChannel, OutputChannel, Process, ProcessKiller, ProcessPty, PtyProcess, SimpleProcess,
    },
    data::{DistantResponseData, ProcessId, PtySize},
};
use distant_net::Reply;
use log::*;
use std::{future::Future, io};
use tokio::task::JoinHandle;

/// Holds information related to a spawned process on the server
pub struct ProcessInstance {
    pub cmd: String,
    pub args: Vec<String>,
    pub persist: bool,

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
        persist: bool,
        pty: Option<PtySize>,
        reply: Box<dyn Reply<Data = DistantResponseData>>,
    ) -> io::Result<Self> {
        // Build out the command and args from our string
        let (cmd, args) = match cmd.split_once(' ') {
            Some((cmd_str, args_str)) => (
                cmd_str.to_string(),
                shell_words::split(args_str)
                    .map_err(|x| io::Error::new(io::ErrorKind::InvalidInput, x))?,
            ),
            None => (cmd, Vec::new()),
        };

        let mut child: Box<dyn Process> = match pty {
            Some(size) => Box::new(PtyProcess::spawn(cmd.clone(), args.clone(), size)?),
            None => Box::new(SimpleProcess::spawn(cmd.clone(), args.clone())?),
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
            persist,
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
    reply: Box<dyn Reply<Data = DistantResponseData>>,
) -> io::Result<()> {
    loop {
        match stdout.recv().await {
            Ok(Some(data)) => {
                if let Err(x) = reply
                    .send(DistantResponseData::ProcStdout { id, data })
                    .await
                {
                    return Err(x);
                }
            }
            Ok(None) => return Ok(()),
            Err(x) => return Err(x),
        }
    }
}

async fn stderr_task(
    id: ProcessId,
    mut stderr: Box<dyn OutputChannel>,
    reply: Box<dyn Reply<Data = DistantResponseData>>,
) -> io::Result<()> {
    loop {
        match stderr.recv().await {
            Ok(Some(data)) => {
                if let Err(x) = reply
                    .send(DistantResponseData::ProcStderr { id, data })
                    .await
                {
                    return Err(x);
                }
            }
            Ok(None) => return Ok(()),
            Err(x) => return Err(x),
        }
    }
}

async fn wait_task(
    id: ProcessId,
    mut child: Box<dyn Process>,
    reply: Box<dyn Reply<Data = DistantResponseData>>,
) -> io::Result<()> {
    let status = child.wait().await;

    match status {
        Ok(status) => {
            reply
                .send(DistantResponseData::ProcDone {
                    id,
                    success: status.success,
                    code: status.code,
                })
                .await
        }
        Err(x) => reply.send(DistantResponseData::from(x)).await,
    }
}
