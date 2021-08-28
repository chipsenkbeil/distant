use crate::core::{
    client::Session,
    constants::CLIENT_BROADCAST_CHANNEL_CAPACITY,
    data::{Request, RequestData, Response, ResponseData},
    net::{DataStream, TransportError},
};
use derive_more::{Display, Error, From};
use tokio::{
    io,
    sync::mpsc,
    task::{JoinError, JoinHandle},
};

#[derive(Debug, Display, Error, From)]
pub enum RemoteProcessError {
    /// When the process receives an unexpected response
    BadResponse,

    /// When attempting to relay stdout/stderr over channels, but the channels fail
    ChannelDead,

    /// When the communication over the wire has issues
    TransportError(TransportError),

    /// When the stream of responses from the server closes without receiving
    /// an indicator of the process' exit status
    UnexpectedEof,

    /// When attempting to wait on the remote process, but the internal task joining failed
    WaitFailed(JoinError),
}

/// Represents a process on a remote machine
pub struct RemoteProcess {
    /// Id of the process
    id: usize,

    /// Task that forwards stdin to the remote process by bundling it as stdin requests
    req_task: JoinHandle<Result<(), RemoteProcessError>>,

    /// Task that reads in new responses, which returns the success and optional
    /// exit code once the process has completed
    res_task: JoinHandle<Result<(bool, Option<i32>), RemoteProcessError>>,

    /// Sender for stdin
    pub stdin: Option<RemoteStdin>,

    /// Receiver for stdout
    pub stdout: Option<RemoteStdout>,

    /// Receiver for stderr
    pub stderr: Option<RemoteStderr>,

    /// Sender for kill events
    kill: mpsc::Sender<()>,
}

impl RemoteProcess {
    /// Spawns the specified process on the remote machine using the given session
    pub async fn spawn<T>(
        tenant: String,
        mut session: Session<T>,
        cmd: String,
        args: Vec<String>,
    ) -> Result<Self, RemoteProcessError>
    where
        T: DataStream + 'static,
    {
        // Submit our run request and wait for a response
        let res = session
            .send(Request::new(
                tenant.as_str(),
                vec![RequestData::ProcRun { cmd, args }],
            ))
            .await?;

        // We expect a singular response back
        if res.payload.len() != 1 {
            return Err(RemoteProcessError::BadResponse);
        }

        // Response should be proc starting
        let id = match res.payload.into_iter().next().unwrap() {
            ResponseData::ProcStart { id } => id,
            _ => return Err(RemoteProcessError::BadResponse),
        };

        // Create channels for our stdin/stdout/stderr
        let (stdin_tx, stdin_rx) = mpsc::channel(CLIENT_BROADCAST_CHANNEL_CAPACITY);
        let (stdout_tx, stdout_rx) = mpsc::channel(CLIENT_BROADCAST_CHANNEL_CAPACITY);
        let (stderr_tx, stderr_rx) = mpsc::channel(CLIENT_BROADCAST_CHANNEL_CAPACITY);

        // Now we spawn a task to handle future responses that are async
        // such as ProcStdout, ProcStderr, and ProcDone
        let broadcast = session.broadcast.take().unwrap();
        let res_task = tokio::spawn(async move {
            process_incoming_responses(id, broadcast, stdout_tx, stderr_tx).await
        });

        // Spawn a task that takes stdin from our channel and forwards it to the remote process
        let (kill_tx, kill_rx) = mpsc::channel(1);
        let req_task = tokio::spawn(async move {
            process_outgoing_requests(tenant, id, session, stdin_rx, kill_rx).await
        });

        Ok(Self {
            id,
            req_task,
            res_task,
            stdin: Some(RemoteStdin(stdin_tx)),
            stdout: Some(RemoteStdout(stdout_rx)),
            stderr: Some(RemoteStderr(stderr_rx)),
            kill: kill_tx,
        })
    }

    /// Returns the id of the running process
    pub fn id(&self) -> usize {
        self.id
    }

    /// Waits for the process to terminate, returning the success status and an optional exit code
    pub async fn wait(self) -> Result<(bool, Option<i32>), RemoteProcessError> {
        match tokio::try_join!(self.req_task, self.res_task) {
            Ok((_, res)) => res,
            Err(x) => Err(RemoteProcessError::from(x)),
        }
    }

    /// Aborts the process by forcing its response task to shutdown, which means that a call
    /// to `wait` will return an error. Note that this does **not** send a kill request, so if
    /// you want to be nice you should send the request before aborting.
    pub fn abort(&self) {
        self.req_task.abort();
        self.res_task.abort();
    }

    /// Submits a kill request for the running process
    pub async fn kill(&mut self) -> Result<(), RemoteProcessError> {
        self.kill
            .send(())
            .await
            .map_err(|_| RemoteProcessError::ChannelDead)?;
        Ok(())
    }
}

/// A handle to a remote process' standard input (stdin)
pub struct RemoteStdin(mpsc::Sender<String>);

impl RemoteStdin {
    /// Writes data to the stdin of a specific remote process
    pub async fn write(&mut self, data: impl Into<String>) -> io::Result<()> {
        self.0
            .send(data.into())
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::BrokenPipe, x))
    }
}

/// A handle to a remote process' standard output (stdout)
pub struct RemoteStdout(mpsc::Receiver<String>);

impl RemoteStdout {
    /// Retrieves the latest stdout for a specific remote process
    pub async fn read(&mut self) -> io::Result<String> {
        self.0
            .recv()
            .await
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))
    }
}

/// A handle to a remote process' stderr
pub struct RemoteStderr(mpsc::Receiver<String>);

impl RemoteStderr {
    /// Retrieves the latest stderr for a specific remote process
    pub async fn read(&mut self) -> io::Result<String> {
        self.0
            .recv()
            .await
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))
    }
}

/// Helper function that loops, processing outgoing stdin requests to a remote process as well as
/// supporting a kill request to terminate the remote process
async fn process_outgoing_requests<T>(
    tenant: String,
    id: usize,
    mut session: Session<T>,
    mut stdin_rx: mpsc::Receiver<String>,
    mut kill_rx: mpsc::Receiver<()>,
) -> Result<(), RemoteProcessError>
where
    T: DataStream,
{
    loop {
        tokio::select! {
            data = stdin_rx.recv() => {
                match data {
                    Some(data) => session.fire(
                        Request::new(
                            tenant.as_str(),
                            vec![RequestData::ProcStdin { id, data }]
                        )
                    ).await?,
                    None => break Err(RemoteProcessError::ChannelDead),
                }
            }
            msg = kill_rx.recv() => {
                if msg.is_some() {
                    session
                        .fire(Request::new(
                            tenant.as_str(),
                            vec![RequestData::ProcKill { id }],
                        ))
                        .await?;
                    break Ok(());
                } else {
                    break Err(RemoteProcessError::ChannelDead);
                }
            }
        }
    }
}

/// Helper function that loops, processing incoming stdout & stderr requests from a remote process
async fn process_incoming_responses(
    proc_id: usize,
    mut broadcast: mpsc::Receiver<Response>,
    stdout_tx: mpsc::Sender<String>,
    stderr_tx: mpsc::Sender<String>,
) -> Result<(bool, Option<i32>), RemoteProcessError> {
    let mut result = Err(RemoteProcessError::UnexpectedEof);

    while let Some(res) = broadcast.recv().await {
        // Check if any of the payload data is the termination
        let exit_status = res.payload.iter().find_map(|data| match data {
            ResponseData::ProcDone { id, success, code } if *id == proc_id => {
                Some((*success, *code))
            }
            _ => None,
        });

        // Next, check for stdout/stderr and send them along our channels
        // TODO: What should we do about unexpected data? For now, just ignore
        for data in res.payload {
            match data {
                ResponseData::ProcStdout { id, data } if id == proc_id => {
                    if let Err(_) = stdout_tx.send(data).await {
                        result = Err(RemoteProcessError::ChannelDead);
                        break;
                    }
                }
                ResponseData::ProcStderr { id, data } if id == proc_id => {
                    if let Err(_) = stderr_tx.send(data).await {
                        result = Err(RemoteProcessError::ChannelDead);
                        break;
                    }
                }
                _ => {}
            }
        }

        // If we got a termination, then exit accordingly
        if let Some((success, code)) = exit_status {
            result = Ok((success, code));
            break;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_should_return_bad_response_if_payload_size_unexpected() {
        todo!();
    }

    #[test]
    fn spawn_should_return_bad_response_if_did_not_get_a_indicator_that_process_started() {
        todo!();
    }

    #[test]
    fn id_should_return_randomly_generated_process_id() {
        todo!();
    }

    #[test]
    fn wait_should_wait_for_internal_tasks_to_complete_and_return_process_exit_information() {
        todo!();
    }

    #[test]
    fn wait_should_return_error_if_internal_tasks_fail() {
        todo!();
    }

    #[test]
    fn abort_should_abort_internal_tasks() {
        todo!();
    }

    #[test]
    fn kill_should_return_error_if_internal_tasks_already_completed() {
        todo!();
    }

    #[test]
    fn kill_should_send_proc_kill_request_and_then_cause_stdin_forwarding_to_close() {
        todo!();
    }

    #[test]
    fn stdin_should_be_forwarded_from_receiver_field() {
        todo!();
    }

    #[test]
    fn stdout_should_be_forwarded_to_receiver_field() {
        todo!();
    }

    #[test]
    fn stderr_should_be_forwarded_to_receiver_field() {
        todo!();
    }

    #[test]
    fn receiving_done_response_should_terminate_internal_tasks() {
        todo!();
    }

    #[test]
    fn receiving_done_response_should_result_in_wait_returning_exit_information() {
        todo!();
    }
}
