use std::future::Future;
use std::io;
use std::path::PathBuf;

use distant_core::net::server::Reply;
use distant_core::protocol::{Environment, ProcessId, PtySize, Response};
use log::*;
use tokio::task::JoinHandle;

use crate::api::process::{
    InputChannel, OutputChannel, Process, ProcessKiller, ProcessPty, PtyProcess, SimpleProcess,
};

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

        debug!("Spawning process: {cmd} {args:?}");
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
                f(task.await.unwrap_or_else(|x| Err(io::Error::other(x)))).await
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
                reply.send(Response::ProcStdout { id, data })?;
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
                reply.send(Response::ProcStderr { id, data })?;
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
        Ok(status) => reply.send(Response::ProcDone {
            id,
            success: status.success,
            code: status.code,
        }),
        Err(x) => reply.send(Response::from(x)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use test_log::test;
    use tokio::sync::mpsc;

    fn make_reply() -> (
        Box<dyn Reply<Data = Response>>,
        mpsc::UnboundedReceiver<Response>,
    ) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Box::new(tx), rx)
    }

    // ---- ProcessInstance::spawn ----

    #[test(tokio::test)]
    async fn spawn_should_fail_with_empty_command() {
        let (reply, _rx) = make_reply();
        let result = ProcessInstance::spawn("".to_string(), Environment::new(), None, None, reply);
        assert!(result.is_err());
        let err = match result {
            Err(e) => e,
            Ok(_) => panic!("Expected error"),
        };
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert!(err.to_string().contains("Command was empty"));
    }

    #[test(tokio::test)]
    async fn spawn_should_succeed_with_simple_command() {
        let (reply, _rx) = make_reply();

        let cmd = if cfg!(windows) {
            "cmd /C echo hello"
        } else {
            "echo hello"
        };

        let instance =
            ProcessInstance::spawn(cmd.to_string(), Environment::new(), None, None, reply).unwrap();

        assert!(!instance.cmd.is_empty());
        assert!(instance.id > 0);
    }

    #[test(tokio::test)]
    async fn spawn_should_parse_command_and_args_correctly() {
        let (reply, _rx) = make_reply();

        let cmd = if cfg!(windows) {
            "cmd /C echo hello"
        } else {
            "echo hello world"
        };

        let instance =
            ProcessInstance::spawn(cmd.to_string(), Environment::new(), None, None, reply).unwrap();

        if cfg!(windows) {
            assert_eq!(instance.cmd, "cmd");
        } else {
            assert_eq!(instance.cmd, "echo");
            assert_eq!(instance.args, vec!["hello", "world"]);
        }
    }

    #[test(tokio::test)]
    async fn spawn_should_fail_with_nonexistent_command() {
        let (reply, _rx) = make_reply();
        let result = ProcessInstance::spawn(
            "nonexistent_command_xyz_123".to_string(),
            Environment::new(),
            None,
            None,
            reply,
        );
        assert!(result.is_err());
    }

    #[test(tokio::test)]
    async fn spawn_should_provide_stdin_for_simple_process() {
        let (reply, _rx) = make_reply();

        let cmd = if cfg!(windows) {
            "cmd /C findstr x]^["
        } else {
            "cat"
        };

        let instance =
            ProcessInstance::spawn(cmd.to_string(), Environment::new(), None, None, reply).unwrap();

        assert!(instance.stdin.is_some());
    }

    #[test(tokio::test)]
    async fn spawn_should_produce_stdout_responses_for_echo() {
        let (reply, mut rx) = make_reply();

        let cmd = if cfg!(windows) {
            "cmd /C echo spawn_test"
        } else {
            "echo spawn_test"
        };

        let _instance =
            ProcessInstance::spawn(cmd.to_string(), Environment::new(), None, None, reply).unwrap();

        // Collect responses until we get ProcDone
        let mut got_stdout = false;
        let mut got_done = false;

        while let Some(resp) = rx.recv().await {
            match resp {
                Response::ProcStdout { data, .. } => {
                    let text = String::from_utf8_lossy(&data);
                    if text.contains("spawn_test") {
                        got_stdout = true;
                    }
                }
                Response::ProcDone { success, .. } => {
                    assert!(success);
                    got_done = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(got_stdout, "Never got stdout with expected content");
        assert!(got_done, "Never got ProcDone");
    }

    #[test(tokio::test)]
    async fn spawn_should_produce_stderr_responses() {
        let (reply, mut rx) = make_reply();

        let cmd = if cfg!(windows) {
            "cmd /C echo stderr_output 1>&2"
        } else {
            "sh -c 'echo stderr_output >&2'"
        };

        let _instance =
            ProcessInstance::spawn(cmd.to_string(), Environment::new(), None, None, reply).unwrap();

        let mut got_stderr = false;
        let mut got_done = false;

        while let Some(resp) = rx.recv().await {
            match resp {
                Response::ProcStderr { data, .. } => {
                    let text = String::from_utf8_lossy(&data);
                    if text.contains("stderr_output") {
                        got_stderr = true;
                    }
                }
                Response::ProcDone { .. } => {
                    got_done = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(got_stderr, "Never got stderr with expected content");
        assert!(got_done, "Never got ProcDone");
    }

    // ---- on_done ----

    #[test(tokio::test)]
    async fn on_done_should_invoke_callback_when_process_finishes() {
        let (reply, _rx) = make_reply();

        let cmd = if cfg!(windows) {
            "cmd /C echo done_test"
        } else {
            "echo done_test"
        };

        let mut instance =
            ProcessInstance::spawn(cmd.to_string(), Environment::new(), None, None, reply).unwrap();

        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let done_tx = Arc::new(Mutex::new(Some(done_tx)));

        instance.on_done(move |result| {
            let done_tx = done_tx.clone();
            async move {
                if let Some(tx) = done_tx.lock().unwrap().take() {
                    let _ = tx.send(result.is_ok());
                }
            }
        });

        // Wait for the callback to be invoked
        let was_ok = tokio::time::timeout(std::time::Duration::from_secs(5), done_rx)
            .await
            .expect("Timed out waiting for on_done")
            .expect("on_done sender dropped");

        assert!(was_ok);
    }

    #[test(tokio::test)]
    async fn on_done_called_twice_should_only_invoke_first_callback() {
        let (reply, _rx) = make_reply();

        let cmd = if cfg!(windows) {
            "cmd /C echo on_done_twice"
        } else {
            "echo on_done_twice"
        };

        let mut instance =
            ProcessInstance::spawn(cmd.to_string(), Environment::new(), None, None, reply).unwrap();

        let (done_tx1, done_rx1) = tokio::sync::oneshot::channel();
        let done_tx1 = Arc::new(Mutex::new(Some(done_tx1)));

        instance.on_done(move |_result| {
            let done_tx = done_tx1.clone();
            async move {
                if let Some(tx) = done_tx.lock().unwrap().take() {
                    let _ = tx.send(true);
                }
            }
        });

        // Second call should be a no-op (wait_task is already taken)
        let (done_tx2, done_rx2) = tokio::sync::oneshot::channel();
        let done_tx2 = Arc::new(Mutex::new(Some(done_tx2)));

        instance.on_done(move |_result| {
            let done_tx = done_tx2.clone();
            async move {
                if let Some(tx) = done_tx.lock().unwrap().take() {
                    let _ = tx.send(true);
                }
            }
        });

        // First callback should fire
        let result1 = tokio::time::timeout(std::time::Duration::from_secs(5), done_rx1)
            .await
            .expect("Timed out waiting for first on_done")
            .expect("first on_done sender dropped");

        assert!(result1);

        // Second callback should NOT fire. Since the closure was dropped
        // immediately (wait_task was already taken), done_tx2 was dropped,
        // causing done_rx2 to resolve with a RecvError.
        let result2 = tokio::time::timeout(std::time::Duration::from_millis(200), done_rx2)
            .await
            .expect("Timed out waiting for second on_done channel to close");

        assert!(
            result2.is_err(),
            "Second on_done callback should not have fired (sender should be dropped)"
        );
    }

    // ---- standalone task functions ----

    #[test(tokio::test)]
    async fn stdout_task_should_send_proc_stdout_responses() {
        let (tx, mut rx) = mpsc::unbounded_channel();

        let (data_tx, data_rx) = mpsc::channel::<Vec<u8>>(10);
        data_tx.send(b"hello".to_vec()).await.unwrap();
        data_tx.send(b"world".to_vec()).await.unwrap();
        drop(data_tx);

        let result = stdout_task(42, Box::new(data_rx), Box::new(tx)).await;
        assert!(result.is_ok());

        let resp1 = rx.recv().await.unwrap();
        match resp1 {
            Response::ProcStdout { id, data } => {
                assert_eq!(id, 42);
                assert_eq!(data, b"hello");
            }
            other => panic!("Expected ProcStdout, got: {other:?}"),
        }

        let resp2 = rx.recv().await.unwrap();
        match resp2 {
            Response::ProcStdout { id, data } => {
                assert_eq!(id, 42);
                assert_eq!(data, b"world");
            }
            other => panic!("Expected ProcStdout, got: {other:?}"),
        }
    }

    #[test(tokio::test)]
    async fn stderr_task_should_send_proc_stderr_responses() {
        let (tx, mut rx) = mpsc::unbounded_channel();

        let (data_tx, data_rx) = mpsc::channel::<Vec<u8>>(10);
        data_tx.send(b"err1".to_vec()).await.unwrap();
        drop(data_tx);

        let result = stderr_task(99, Box::new(data_rx), Box::new(tx)).await;
        assert!(result.is_ok());

        let resp = rx.recv().await.unwrap();
        match resp {
            Response::ProcStderr { id, data } => {
                assert_eq!(id, 99);
                assert_eq!(data, b"err1");
            }
            other => panic!("Expected ProcStderr, got: {other:?}"),
        }
    }

    #[test(tokio::test)]
    async fn stdout_task_should_return_error_when_reply_channel_closed() {
        let (tx, rx) = mpsc::unbounded_channel();
        drop(rx); // Close the receiver immediately

        let (data_tx, data_rx) = mpsc::channel::<Vec<u8>>(10);
        data_tx.send(b"data".to_vec()).await.unwrap();

        // Should get an error when trying to send the response
        let result = stdout_task(1, Box::new(data_rx), Box::new(tx)).await;
        assert!(result.is_err());
    }

    #[test(tokio::test)]
    async fn stderr_task_should_return_error_when_reply_channel_closed() {
        let (tx, rx) = mpsc::unbounded_channel();
        drop(rx);

        let (data_tx, data_rx) = mpsc::channel::<Vec<u8>>(10);
        data_tx.send(b"data".to_vec()).await.unwrap();

        let result = stderr_task(1, Box::new(data_rx), Box::new(tx)).await;
        assert!(result.is_err());
    }

    #[test(tokio::test)]
    async fn stdout_task_should_propagate_output_channel_errors() {
        use crate::api::process::{FutureReturn, OutputChannel};

        struct ErrorChannel;
        impl OutputChannel for ErrorChannel {
            fn recv(&mut self) -> FutureReturn<'_, io::Result<Option<Vec<u8>>>> {
                Box::pin(async { Err(io::Error::other("read error")) })
            }
        }

        let (tx, _rx) = mpsc::unbounded_channel();
        let result = stdout_task(1, Box::new(ErrorChannel), Box::new(tx)).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("read error"));
    }

    #[test(tokio::test)]
    async fn stderr_task_should_propagate_output_channel_errors() {
        use crate::api::process::{FutureReturn, OutputChannel};

        struct ErrorChannel;
        impl OutputChannel for ErrorChannel {
            fn recv(&mut self) -> FutureReturn<'_, io::Result<Option<Vec<u8>>>> {
                Box::pin(async { Err(io::Error::other("stderr error")) })
            }
        }

        let (tx, _rx) = mpsc::unbounded_channel();
        let result = stderr_task(1, Box::new(ErrorChannel), Box::new(tx)).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("stderr error"));
    }

    #[test(tokio::test)]
    async fn spawn_should_set_current_dir_when_specified() {
        let (reply, mut rx) = make_reply();

        let temp_dir = std::env::temp_dir();
        let cmd = if cfg!(windows) { "cmd /C cd" } else { "pwd" };

        let _instance = ProcessInstance::spawn(
            cmd.to_string(),
            Environment::new(),
            Some(temp_dir.clone()),
            None,
            reply,
        )
        .unwrap();

        // Collect stdout to verify the current directory
        let mut stdout_data = Vec::new();
        while let Some(resp) = rx.recv().await {
            match resp {
                Response::ProcStdout { data, .. } => {
                    stdout_data.extend_from_slice(&data);
                }
                Response::ProcDone { .. } => break,
                _ => {}
            }
        }

        let output = String::from_utf8_lossy(&stdout_data);
        let output_path = std::path::PathBuf::from(output.trim());
        let canonical_output = output_path.canonicalize().unwrap_or(output_path);
        let canonical_temp = temp_dir.canonicalize().unwrap_or(temp_dir);
        assert_eq!(
            canonical_output, canonical_temp,
            "Output dir '{}' did not match expected temp dir '{}'",
            canonical_output.display(),
            canonical_temp.display()
        );
    }
}
