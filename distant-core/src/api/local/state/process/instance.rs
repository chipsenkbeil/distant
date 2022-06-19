use crate::{
    api::local::process::{
        InputChannel, OutputChannel, Process, ProcessKiller, ProcessPty, PtyProcess, SimpleProcess,
    },
    data::{DistantResponseData, PtySize},
};
use distant_net::QueuedServerReply;
use log::*;
use std::io;

/// Holds information related to a spawned process on the server
pub struct ProcessInstance {
    pub cmd: String,
    pub args: Vec<String>,
    pub persist: bool,

    pub id: usize,
    pub stdin: Option<Box<dyn InputChannel>>,
    pub killer: Box<dyn ProcessKiller>,
    pub pty: Box<dyn ProcessPty>,
}

impl Drop for ProcessInstance {
    /// Closes stdin and attempts to kill the process when dropped
    fn drop(&mut self) {
        // Drop stdin first to close it
        self.stdin = None;

        // Attempt to kill the process, which is an async operation that we
        // will spawn a task to handle
        let id = self.id;
        let killer = self.killer.clone_killer();
        tokio::spawn(async move {
            if let Err(x) = killer.kill().await {
                error!("Failed to kill process {} when dropped: {}", id, x);
            }
        });
    }
}

impl ProcessInstance {
    pub fn spawn(
        cmd: String,
        persist: bool,
        pty: Option<PtySize>,
        reply: QueuedServerReply<DistantResponseData>,
    ) -> io::Result<Self> {
        // Build out the command and args from our string
        let (cmd, args) = match cmd.split_once(" ") {
            Some((cmd_str, args_str)) => (
                cmd.to_string(),
                args_str.split(" ").map(ToString::to_string).collect(),
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
        if let Some(stdout) = stdout {
            let reply = reply.clone();
            let _ = tokio::spawn(async move { stdout_task(id, stdout, reply).await });
        }

        // Spawn a task that sends stderr as a response
        if let Some(stderr) = stderr {
            let reply = reply.clone();
            let _ = tokio::spawn(async move { stderr_task(id, stderr, reply).await });
        }

        // Spawn a task that waits on the process to exit but can also
        // kill the process when triggered
        let _ = tokio::spawn(async move { wait_task(id, child, reply).await });

        Ok(ProcessInstance {
            cmd,
            args,
            persist,
            id,
            stdin,
            killer,
            pty,
        })
    }
}

async fn stdout_task(
    id: usize,
    mut stdout: Box<dyn OutputChannel>,
    reply: QueuedServerReply<DistantResponseData>,
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
    id: usize,
    mut stderr: Box<dyn OutputChannel>,
    reply: QueuedServerReply<DistantResponseData>,
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
    id: usize,
    mut child: Box<dyn Process>,
    reply: QueuedServerReply<DistantResponseData>,
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
