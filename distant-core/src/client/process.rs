use crate::{
    client::DistantChannel,
    constants::CLIENT_PIPE_CAPACITY,
    data::{Cmd, DistantRequestData, DistantResponseData, Environment, ProcessId, PtySize},
    DistantMsg,
};
use distant_net::{
    client::Mailbox,
    common::{Request, Response},
};
use log::*;
use std::{path::PathBuf, sync::Arc};
use tokio::{
    io,
    sync::{
        mpsc::{
            self,
            error::{TryRecvError, TrySendError},
        },
        RwLock,
    },
    task::JoinHandle,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteOutput {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct RemoteStatus {
    pub success: bool,
    pub code: Option<i32>,
}

impl From<(bool, Option<i32>)> for RemoteStatus {
    fn from((success, code): (bool, Option<i32>)) -> Self {
        Self { success, code }
    }
}

type StatusResult = io::Result<RemoteStatus>;

/// A [`RemoteProcess`] builder providing support to configure
/// before spawning the process on a remote machine
pub struct RemoteCommand {
    persist: bool,
    pty: Option<PtySize>,
    environment: Environment,
    current_dir: Option<PathBuf>,
}

impl Default for RemoteCommand {
    fn default() -> Self {
        Self::new()
    }
}

impl RemoteCommand {
    /// Creates a new set of options for a remote process
    pub fn new() -> Self {
        Self {
            persist: false,
            pty: None,
            environment: Environment::new(),
            current_dir: None,
        }
    }

    /// Sets whether or not the process will be persistent,
    /// meaning that it will not be terminated when the
    /// connection to the remote machine is terminated
    pub fn persist(&mut self, persist: bool) -> &mut Self {
        self.persist = persist;
        self
    }

    /// Configures the process to leverage a PTY with the specified size
    pub fn pty(&mut self, pty: Option<PtySize>) -> &mut Self {
        self.pty = pty;
        self
    }

    /// Replaces the existing environment variables with the given collection
    pub fn environment(&mut self, environment: Environment) -> &mut Self {
        self.environment = environment;
        self
    }

    /// Configures the process with an alternative current directory
    pub fn current_dir(&mut self, current_dir: Option<PathBuf>) -> &mut Self {
        self.current_dir = current_dir;
        self
    }

    /// Spawns the specified process on the remote machine using the given `channel` and `cmd`
    pub async fn spawn(
        &mut self,
        mut channel: DistantChannel,
        cmd: impl Into<String>,
    ) -> io::Result<RemoteProcess> {
        let cmd = cmd.into();

        // Submit our run request and get back a mailbox for responses
        let mut mailbox = channel
            .mail(Request::new(DistantMsg::Single(
                DistantRequestData::ProcSpawn {
                    cmd: Cmd::from(cmd),
                    persist: self.persist,
                    pty: self.pty,
                    environment: self.environment.clone(),
                    current_dir: self.current_dir.clone(),
                },
            )))
            .await?;

        // Wait until we get the first response, and get id from proc started
        let (id, origin_id) = match mailbox.next().await {
            Some(res) => {
                let origin_id = res.origin_id;
                match res.payload {
                    DistantMsg::Single(DistantResponseData::ProcSpawned { id }) => (id, origin_id),
                    DistantMsg::Single(DistantResponseData::Error(x)) => return Err(x.into()),
                    DistantMsg::Single(x) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("Got response type of {}", x.as_ref()),
                        ))
                    }
                    DistantMsg::Batch(_) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "Got batch instead of single response",
                        ));
                    }
                }
            }
            None => return Err(io::Error::from(io::ErrorKind::ConnectionAborted)),
        };

        // Create channels for our stdin/stdout/stderr
        let (stdin_tx, stdin_rx) = mpsc::channel(CLIENT_PIPE_CAPACITY);
        let (stdout_tx, stdout_rx) = mpsc::channel(CLIENT_PIPE_CAPACITY);
        let (stderr_tx, stderr_rx) = mpsc::channel(CLIENT_PIPE_CAPACITY);
        let (resize_tx, resize_rx) = mpsc::channel(1);

        // Used to terminate request task, either explicitly by the process or internally
        // by the response task when it terminates
        let (kill_tx, kill_rx) = mpsc::channel(1);
        let kill_tx_2 = kill_tx.clone();

        // Now we spawn a task to handle future responses that are async
        // such as ProcStdout, ProcStderr, and ProcDone
        let (abort_res_task_tx, mut abort_res_task_rx) = mpsc::channel::<()>(1);
        let res_task = tokio::spawn(async move {
            tokio::select! {
                _ = abort_res_task_rx.recv() => {
                    panic!("killed");
                }
                res = process_incoming_responses(id, mailbox, stdout_tx, stderr_tx, kill_tx_2) => {
                    res
                }
            }
        });

        // Spawn a task that takes stdin from our channel and forwards it to the remote process
        let (abort_req_task_tx, mut abort_req_task_rx) = mpsc::channel::<()>(1);
        let req_task = tokio::spawn(async move {
            tokio::select! {
                _ = abort_req_task_rx.recv() => {
                    panic!("killed");
                }
                res = process_outgoing_requests( id, channel, stdin_rx, resize_rx, kill_rx) => {
                    res
                }
            }
        });

        let status = Arc::new(RwLock::new(None));
        let status_2 = Arc::clone(&status);
        let wait_task = tokio::spawn(async move {
            let res = match tokio::try_join!(req_task, res_task) {
                Ok((_, res)) => res.map(RemoteStatus::from),
                Err(x) => Err(io::Error::new(io::ErrorKind::Interrupted, x)),
            };
            status_2.write().await.replace(res);
        });

        Ok(RemoteProcess {
            id,
            origin_id,
            abort_req_task_tx,
            abort_res_task_tx,
            stdin: Some(RemoteStdin(stdin_tx)),
            stdout: Some(RemoteStdout(stdout_rx)),
            stderr: Some(RemoteStderr(stderr_rx)),
            resizer: RemoteProcessResizer(resize_tx),
            killer: RemoteProcessKiller(kill_tx),
            wait_task,
            status,
        })
    }
}

/// Represents a process on a remote machine
#[derive(Debug)]
pub struct RemoteProcess {
    /// Id of the process
    id: ProcessId,

    /// Id used to map back to mailbox
    origin_id: String,

    // Sender to abort req task
    abort_req_task_tx: mpsc::Sender<()>,

    // Sender to abort res task
    abort_res_task_tx: mpsc::Sender<()>,

    /// Sender for stdin
    pub stdin: Option<RemoteStdin>,

    /// Receiver for stdout
    pub stdout: Option<RemoteStdout>,

    /// Receiver for stderr
    pub stderr: Option<RemoteStderr>,

    /// Sender for resize events
    resizer: RemoteProcessResizer,

    /// Sender for kill events
    killer: RemoteProcessKiller,

    /// Task that waits for the process to complete
    wait_task: JoinHandle<()>,

    /// Handles the success and exit code for a completed process
    status: Arc<RwLock<Option<StatusResult>>>,
}

impl RemoteProcess {
    /// Returns the id of the running process
    pub fn id(&self) -> ProcessId {
        self.id
    }

    /// Returns the id of the request that spawned this process
    pub fn origin_id(&self) -> &str {
        &self.origin_id
    }

    /// Checks if the process has completed, returning the exit status if it has, without
    /// consuming the process itself. Note that this does not include join errors that can
    /// occur when aborting and instead converts any error to a status of false. To acquire
    /// the actual error, you must call `wait`
    pub async fn status(&self) -> Option<RemoteStatus> {
        self.status.read().await.as_ref().map(|x| match x {
            Ok(status) => *status,
            Err(_) => RemoteStatus {
                success: false,
                code: None,
            },
        })
    }

    /// Waits for the process to terminate, returning the success status and an optional exit code
    pub async fn wait(self) -> io::Result<RemoteStatus> {
        // Wait for the process to complete before we try to get the status
        let _ = self.wait_task.await;

        // NOTE: If we haven't received an exit status, this lines up with the UnexpectedEof error
        self.status
            .write()
            .await
            .take()
            .unwrap_or_else(|| Err(errors::unexpected_eof()))
    }

    /// Waits for the process to terminate, returning the success status, an optional exit code,
    /// and any remaining stdout and stderr (if still attached to the process)
    pub async fn output(mut self) -> io::Result<RemoteOutput> {
        let maybe_stdout = self.stdout.take();
        let maybe_stderr = self.stderr.take();

        let status = self.wait().await?;

        let mut stdout = Vec::new();
        if let Some(mut reader) = maybe_stdout {
            while let Ok(data) = reader.read().await {
                stdout.extend(&data);
            }
        }

        let mut stderr = Vec::new();
        if let Some(mut reader) = maybe_stderr {
            while let Ok(data) = reader.read().await {
                stderr.extend(&data);
            }
        }

        Ok(RemoteOutput {
            success: status.success,
            code: status.code,
            stdout,
            stderr,
        })
    }

    /// Resizes the pty of the remote process if it is attached to one
    pub async fn resize(&self, size: PtySize) -> io::Result<()> {
        self.resizer.resize(size).await
    }

    /// Clones a copy of the remote process pty resizer
    pub fn clone_resizer(&self) -> RemoteProcessResizer {
        self.resizer.clone()
    }

    /// Submits a kill request for the running process
    pub async fn kill(&mut self) -> io::Result<()> {
        self.killer.kill().await
    }

    /// Clones a copy of the remote process killer
    pub fn clone_killer(&self) -> RemoteProcessKiller {
        self.killer.clone()
    }

    /// Aborts the process by forcing its response task to shutdown, which means that a call
    /// to `wait` will return an error. Note that this does **not** send a kill request, so if
    /// you want to be nice you should send the request before aborting.
    pub fn abort(&self) {
        let _ = self.abort_req_task_tx.try_send(());
        let _ = self.abort_res_task_tx.try_send(());
    }
}

/// A handle to the channel to kill a remote process
#[derive(Clone, Debug)]
pub struct RemoteProcessResizer(mpsc::Sender<PtySize>);

impl RemoteProcessResizer {
    /// Resizes the pty of the remote process if it is attached to one
    pub async fn resize(&self, size: PtySize) -> io::Result<()> {
        self.0
            .send(size)
            .await
            .map_err(|_| errors::dead_channel())?;
        Ok(())
    }
}

/// A handle to the channel to kill a remote process
#[derive(Clone, Debug)]
pub struct RemoteProcessKiller(mpsc::Sender<()>);

impl RemoteProcessKiller {
    /// Submits a kill request for the running process
    pub async fn kill(&mut self) -> io::Result<()> {
        self.0.send(()).await.map_err(|_| errors::dead_channel())?;
        Ok(())
    }
}

/// A handle to a remote process' standard input (stdin)
#[derive(Clone, Debug)]
pub struct RemoteStdin(mpsc::Sender<Vec<u8>>);

impl RemoteStdin {
    /// Creates a disconnected remote stdin
    pub fn disconnected() -> Self {
        Self(mpsc::channel(1).0)
    }

    /// Tries to write to the stdin of the remote process, returning ok if immediately
    /// successful, `WouldBlock` if would need to wait to send data, and `BrokenPipe`
    /// if stdin has been closed
    pub fn try_write(&mut self, data: impl Into<Vec<u8>>) -> io::Result<()> {
        match self.0.try_send(data.into()) {
            Ok(data) => Ok(data),
            Err(TrySendError::Full(_)) => Err(io::Error::from(io::ErrorKind::WouldBlock)),
            Err(TrySendError::Closed(_)) => Err(io::Error::from(io::ErrorKind::BrokenPipe)),
        }
    }

    /// Same as `try_write`, but with a string
    pub fn try_write_str(&mut self, data: impl Into<String>) -> io::Result<()> {
        self.try_write(data.into().into_bytes())
    }

    /// Writes data to the stdin of a specific remote process
    pub async fn write(&mut self, data: impl Into<Vec<u8>>) -> io::Result<()> {
        self.0
            .send(data.into())
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::BrokenPipe, x))
    }

    /// Same as `write`, but with a string
    pub async fn write_str(&mut self, data: impl Into<String>) -> io::Result<()> {
        self.write(data.into().into_bytes()).await
    }

    /// Checks if stdin has been closed
    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }
}

/// A handle to a remote process' standard output (stdout)
#[derive(Debug)]
pub struct RemoteStdout(mpsc::Receiver<Vec<u8>>);

impl RemoteStdout {
    /// Tries to receive latest stdout for a remote process, yielding `None`
    /// if no stdout is available, and `BrokenPipe` if stdout has been closed
    pub fn try_read(&mut self) -> io::Result<Option<Vec<u8>>> {
        match self.0.try_recv() {
            Ok(data) => Ok(Some(data)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(io::Error::from(io::ErrorKind::BrokenPipe)),
        }
    }

    /// Same as `try_read`, but returns a string
    pub fn try_read_string(&mut self) -> io::Result<Option<String>> {
        self.try_read().and_then(|x| match x {
            Some(data) => String::from_utf8(data)
                .map(Some)
                .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x)),
            None => Ok(None),
        })
    }

    /// Retrieves the latest stdout for a specific remote process, and `BrokenPipe` if stdout has
    /// been closed
    pub async fn read(&mut self) -> io::Result<Vec<u8>> {
        self.0
            .recv()
            .await
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))
    }

    /// Same as `read`, but returns a string
    pub async fn read_string(&mut self) -> io::Result<String> {
        self.read().await.and_then(|data| {
            String::from_utf8(data).map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))
        })
    }
}

/// A handle to a remote process' stderr
#[derive(Debug)]
pub struct RemoteStderr(mpsc::Receiver<Vec<u8>>);

impl RemoteStderr {
    /// Tries to receive latest stderr for a remote process, yielding `None`
    /// if no stderr is available, and `BrokenPipe` if stderr has been closed
    pub fn try_read(&mut self) -> io::Result<Option<Vec<u8>>> {
        match self.0.try_recv() {
            Ok(data) => Ok(Some(data)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(io::Error::from(io::ErrorKind::BrokenPipe)),
        }
    }

    /// Same as `try_read`, but returns a string
    pub fn try_read_string(&mut self) -> io::Result<Option<String>> {
        self.try_read().and_then(|x| match x {
            Some(data) => String::from_utf8(data)
                .map(Some)
                .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x)),
            None => Ok(None),
        })
    }

    /// Retrieves the latest stderr for a specific remote process, and `BrokenPipe` if stderr has
    /// been closed
    pub async fn read(&mut self) -> io::Result<Vec<u8>> {
        self.0
            .recv()
            .await
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))
    }

    /// Same as `read`, but returns a string
    pub async fn read_string(&mut self) -> io::Result<String> {
        self.read().await.and_then(|data| {
            String::from_utf8(data).map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))
        })
    }
}

/// Helper function that loops, processing outgoing stdin requests to a remote process as well as
/// supporting a kill request to terminate the remote process
async fn process_outgoing_requests(
    id: ProcessId,
    mut channel: DistantChannel,
    mut stdin_rx: mpsc::Receiver<Vec<u8>>,
    mut resize_rx: mpsc::Receiver<PtySize>,
    mut kill_rx: mpsc::Receiver<()>,
) -> io::Result<()> {
    let result = loop {
        tokio::select! {
            data = stdin_rx.recv() => {
                match data {
                    Some(data) => channel.fire(
                        Request::new(
                            DistantMsg::Single(DistantRequestData::ProcStdin { id, data })
                        )
                    ).await?,
                    None => break Err(errors::dead_channel()),
                }
            }
            size = resize_rx.recv() => {
                match size {
                    Some(size) => channel.fire(
                        Request::new(
                            DistantMsg::Single(DistantRequestData::ProcResizePty { id, size })
                        )
                    ).await?,
                    None => break Err(errors::dead_channel()),
                }
            }
            msg = kill_rx.recv() => {
                if msg.is_some() {
                    channel.fire(Request::new(
                        DistantMsg::Single(DistantRequestData::ProcKill { id })
                    )).await?;
                    break Ok(());
                } else {
                    break Err(errors::dead_channel());
                }
            }
        }
    };

    trace!("Process outgoing channel closed");
    result
}

/// Helper function that loops, processing incoming stdout & stderr requests from a remote process
async fn process_incoming_responses(
    proc_id: ProcessId,
    mut mailbox: Mailbox<Response<DistantMsg<DistantResponseData>>>,
    stdout_tx: mpsc::Sender<Vec<u8>>,
    stderr_tx: mpsc::Sender<Vec<u8>>,
    kill_tx: mpsc::Sender<()>,
) -> io::Result<(bool, Option<i32>)> {
    while let Some(res) = mailbox.next().await {
        let payload = res.payload.into_vec();

        // Check if any of the payload data is the termination
        let exit_status = payload.iter().find_map(|data| match data {
            DistantResponseData::ProcDone { id, success, code } if *id == proc_id => {
                Some((*success, *code))
            }
            _ => None,
        });

        // Next, check for stdout/stderr and send them along our channels
        // TODO: What should we do about unexpected data? For now, just ignore
        for data in payload {
            match data {
                DistantResponseData::ProcStdout { id, data } if id == proc_id => {
                    let _ = stdout_tx.send(data).await;
                }
                DistantResponseData::ProcStderr { id, data } if id == proc_id => {
                    let _ = stderr_tx.send(data).await;
                }
                _ => {}
            }
        }

        // If we got a termination, then exit accordingly
        if let Some((success, code)) = exit_status {
            // Flag that the other task should conclude
            let _ = kill_tx.try_send(());

            return Ok((success, code));
        }
    }

    // Flag that the other task should conclude
    let _ = kill_tx.try_send(());

    trace!("Process incoming channel closed");
    Err(errors::unexpected_eof())
}

mod errors {
    use std::io;

    pub fn dead_channel() -> io::Error {
        io::Error::new(io::ErrorKind::BrokenPipe, "Channel is dead")
    }

    pub fn unexpected_eof() -> io::Error {
        io::Error::from(io::ErrorKind::UnexpectedEof)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::DistantClient,
        data::{Error, ErrorKind},
    };
    use distant_net::{
        common::{FramedTransport, InmemoryTransport, Response},
        Client,
    };
    use std::time::Duration;
    use test_log::test;

    fn make_session() -> (FramedTransport<InmemoryTransport>, DistantClient) {
        let (t1, t2) = FramedTransport::pair(100);
        (t1, Client::spawn_inmemory(t2, Default::default()))
    }

    #[test(tokio::test)]
    async fn spawn_should_return_invalid_data_if_received_batch_response() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteCommand::new()
                .spawn(session.clone_channel(), String::from("cmd arg"))
                .await
        });

        // Wait until we get the request from the session
        let req: Request<DistantMsg<DistantRequestData>> =
            transport.read_frame_as().await.unwrap().unwrap();

        // Send back a response through the session
        transport
            .write_frame_for(&Response::new(
                req.id,
                DistantMsg::Batch(vec![DistantResponseData::ProcSpawned { id: 1 }]),
            ))
            .await
            .unwrap();

        // Get the spawn result and verify
        match spawn_task.await.unwrap() {
            Err(x) if x.kind() == io::ErrorKind::InvalidData => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[test(tokio::test)]
    async fn spawn_should_return_invalid_data_if_did_not_get_a_indicator_that_process_started() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteCommand::new()
                .spawn(session.clone_channel(), String::from("cmd arg"))
                .await
        });

        // Wait until we get the request from the session
        let req: Request<DistantMsg<DistantRequestData>> =
            transport.read_frame_as().await.unwrap().unwrap();

        // Send back a response through the session
        transport
            .write_frame_for(&Response::new(
                req.id,
                DistantMsg::Single(DistantResponseData::Error(Error {
                    kind: ErrorKind::BrokenPipe,
                    description: String::from("some error"),
                })),
            ))
            .await
            .unwrap();

        // Get the spawn result and verify
        match spawn_task.await.unwrap() {
            Err(x) if x.kind() == io::ErrorKind::BrokenPipe => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[test(tokio::test)]
    async fn kill_should_return_error_if_internal_tasks_already_completed() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteCommand::new()
                .spawn(session.clone_channel(), String::from("cmd arg"))
                .await
        });

        // Wait until we get the request from the session
        let req: Request<DistantMsg<DistantRequestData>> =
            transport.read_frame_as().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .write_frame_for(&Response::new(
                req.id,
                DistantMsg::Single(DistantResponseData::ProcSpawned { id }),
            ))
            .await
            .unwrap();

        // Receive the process and then abort it to make kill fail
        let mut proc = spawn_task.await.unwrap().unwrap();
        proc.abort();

        // Ensure that the other tasks are aborted before continuing
        tokio::task::yield_now().await;

        match proc.kill().await {
            Err(x) if x.kind() == io::ErrorKind::BrokenPipe => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[test(tokio::test)]
    async fn kill_should_send_proc_kill_request_and_then_cause_stdin_forwarding_to_close() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteCommand::new()
                .spawn(session.clone_channel(), String::from("cmd arg"))
                .await
        });

        // Wait until we get the request from the session
        let req: Request<DistantMsg<DistantRequestData>> =
            transport.read_frame_as().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .write_frame_for(&Response::new(
                req.id,
                DistantMsg::Single(DistantResponseData::ProcSpawned { id }),
            ))
            .await
            .unwrap();

        // Receive the process and then kill it
        let mut proc = spawn_task.await.unwrap().unwrap();
        assert!(proc.kill().await.is_ok(), "Failed to send kill request");

        // Verify the kill request was sent
        let req: Request<DistantMsg<DistantRequestData>> =
            transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            DistantMsg::Single(DistantRequestData::ProcKill { id: proc_id }) => {
                assert_eq!(proc_id, id)
            }
            x => panic!("Unexpected request: {:?}", x),
        }

        // Verify we can no longer write to stdin anymore
        assert_eq!(
            proc.stdin
                .as_mut()
                .unwrap()
                .write("some stdin")
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::BrokenPipe
        );
    }

    #[test(tokio::test)]
    async fn stdin_should_be_forwarded_from_receiver_field() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteCommand::new()
                .spawn(session.clone_channel(), String::from("cmd arg"))
                .await
        });

        // Wait until we get the request from the session
        let req: Request<DistantMsg<DistantRequestData>> =
            transport.read_frame_as().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .write_frame_for(&Response::new(
                req.id,
                DistantMsg::Single(DistantResponseData::ProcSpawned { id }),
            ))
            .await
            .unwrap();

        // Receive the process and then send stdin
        let mut proc = spawn_task.await.unwrap().unwrap();
        proc.stdin
            .as_mut()
            .unwrap()
            .write("some input")
            .await
            .unwrap();

        // Verify that a request is made through the session
        let req: Request<DistantMsg<DistantRequestData>> =
            transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            DistantMsg::Single(DistantRequestData::ProcStdin { id, data }) => {
                assert_eq!(id, 12345);
                assert_eq!(data, b"some input");
            }
            x => panic!("Unexpected request: {:?}", x),
        }
    }

    #[test(tokio::test)]
    async fn stdout_should_be_forwarded_to_receiver_field() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteCommand::new()
                .spawn(session.clone_channel(), String::from("cmd arg"))
                .await
        });

        // Wait until we get the request from the session
        let req: Request<DistantMsg<DistantRequestData>> =
            transport.read_frame_as().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .write_frame_for(&Response::new(
                req.id.clone(),
                DistantMsg::Single(DistantResponseData::ProcSpawned { id }),
            ))
            .await
            .unwrap();

        // Receive the process and then read stdout
        let mut proc = spawn_task.await.unwrap().unwrap();

        transport
            .write_frame_for(&Response::new(
                req.id,
                DistantMsg::Single(DistantResponseData::ProcStdout {
                    id,
                    data: b"some out".to_vec(),
                }),
            ))
            .await
            .unwrap();

        let out = proc.stdout.as_mut().unwrap().read().await.unwrap();
        assert_eq!(out, b"some out");
    }

    #[test(tokio::test)]
    async fn stderr_should_be_forwarded_to_receiver_field() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteCommand::new()
                .spawn(session.clone_channel(), String::from("cmd arg"))
                .await
        });

        // Wait until we get the request from the session
        let req: Request<DistantMsg<DistantRequestData>> =
            transport.read_frame_as().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .write_frame_for(&Response::new(
                req.id.clone(),
                DistantMsg::Single(DistantResponseData::ProcSpawned { id }),
            ))
            .await
            .unwrap();

        // Receive the process and then read stderr
        let mut proc = spawn_task.await.unwrap().unwrap();

        transport
            .write_frame_for(&Response::new(
                req.id,
                DistantMsg::Single(DistantResponseData::ProcStderr {
                    id,
                    data: b"some err".to_vec(),
                }),
            ))
            .await
            .unwrap();

        let out = proc.stderr.as_mut().unwrap().read().await.unwrap();
        assert_eq!(out, b"some err");
    }

    #[test(tokio::test)]
    async fn status_should_return_none_if_not_done() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteCommand::new()
                .spawn(session.clone_channel(), String::from("cmd arg"))
                .await
        });

        // Wait until we get the request from the session
        let req: Request<DistantMsg<DistantRequestData>> =
            transport.read_frame_as().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .write_frame_for(&Response::new(
                req.id,
                DistantMsg::Single(DistantResponseData::ProcSpawned { id }),
            ))
            .await
            .unwrap();

        // Receive the process and then check its status
        let proc = spawn_task.await.unwrap().unwrap();

        let result = proc.status().await;
        assert_eq!(result, None, "Unexpectedly got proc status: {:?}", result);
    }

    #[test(tokio::test)]
    async fn status_should_return_false_for_success_if_internal_tasks_fail() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteCommand::new()
                .spawn(session.clone_channel(), String::from("cmd arg"))
                .await
        });

        // Wait until we get the request from the session
        let req: Request<DistantMsg<DistantRequestData>> =
            transport.read_frame_as().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .write_frame_for(&Response::new(
                req.id,
                DistantMsg::Single(DistantResponseData::ProcSpawned { id }),
            ))
            .await
            .unwrap();

        // Receive the process and then abort it to make internal tasks fail
        let proc = spawn_task.await.unwrap().unwrap();
        proc.abort();

        // Wait a bit to ensure the other tasks abort
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Peek at the status to confirm the result
        let result = proc.status().await;
        match result {
            Some(status) => {
                assert!(!status.success, "Status unexpectedly reported success");
                assert!(
                    status.code.is_none(),
                    "Status unexpectedly reported exit code"
                );
            }
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[test(tokio::test)]
    async fn status_should_return_process_status_when_done() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteCommand::new()
                .spawn(session.clone_channel(), String::from("cmd arg"))
                .await
        });

        // Wait until we get the request from the session
        let req: Request<DistantMsg<DistantRequestData>> =
            transport.read_frame_as().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .write_frame_for(&Response::new(
                req.id.clone(),
                DistantMsg::Single(DistantResponseData::ProcSpawned { id }),
            ))
            .await
            .unwrap();

        // Receive the process and then spawn a task for it to complete
        let proc = spawn_task.await.unwrap().unwrap();

        // Send a process completion response to pass along exit status and conclude wait
        transport
            .write_frame_for(&Response::new(
                req.id,
                DistantMsg::Single(DistantResponseData::ProcDone {
                    id,
                    success: true,
                    code: Some(123),
                }),
            ))
            .await
            .unwrap();

        // Wait a bit to ensure the status gets transmitted
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Finally, verify that we complete and get the expected results
        assert_eq!(
            proc.status().await,
            Some(RemoteStatus {
                success: true,
                code: Some(123)
            })
        );
    }

    #[test(tokio::test)]
    async fn wait_should_return_error_if_internal_tasks_fail() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteCommand::new()
                .spawn(session.clone_channel(), String::from("cmd arg"))
                .await
        });

        // Wait until we get the request from the session
        let req: Request<DistantMsg<DistantRequestData>> =
            transport.read_frame_as().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .write_frame_for(&Response::new(
                req.id,
                DistantMsg::Single(DistantResponseData::ProcSpawned { id }),
            ))
            .await
            .unwrap();

        // Receive the process and then abort it to make internal tasks fail
        let proc = spawn_task.await.unwrap().unwrap();
        proc.abort();

        match proc.wait().await {
            Err(x) if x.kind() == io::ErrorKind::Interrupted => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[test(tokio::test)]
    async fn wait_should_return_error_if_connection_terminates_before_receiving_done_response() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteCommand::new()
                .spawn(session.clone_channel(), String::from("cmd arg"))
                .await
        });

        // Wait until we get the request from the session
        let req: Request<DistantMsg<DistantRequestData>> =
            transport.read_frame_as().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .write_frame_for(&Response::new(
                req.id,
                DistantMsg::Single(DistantResponseData::ProcSpawned { id }),
            ))
            .await
            .unwrap();

        // Receive the process and then terminate session connection
        let proc = spawn_task.await.unwrap().unwrap();

        // Ensure that the spawned task gets a chance to wait on stdout/stderr
        tokio::task::yield_now().await;

        drop(transport);

        // Ensure that the other tasks are cancelled before continuing
        tokio::task::yield_now().await;

        match proc.wait().await {
            Err(x) if x.kind() == io::ErrorKind::UnexpectedEof => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[test(tokio::test)]
    async fn receiving_done_response_should_result_in_wait_returning_exit_information() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteCommand::new()
                .spawn(session.clone_channel(), String::from("cmd arg"))
                .await
        });

        // Wait until we get the request from the session
        let req: Request<DistantMsg<DistantRequestData>> =
            transport.read_frame_as().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .write_frame_for(&Response::new(
                req.id.clone(),
                DistantMsg::Single(DistantResponseData::ProcSpawned { id }),
            ))
            .await
            .unwrap();

        // Receive the process and then spawn a task for it to complete
        let proc = spawn_task.await.unwrap().unwrap();
        let proc_wait_task = tokio::spawn(proc.wait());

        // Send a process completion response to pass along exit status and conclude wait
        transport
            .write_frame_for(&Response::new(
                req.id,
                DistantMsg::Single(DistantResponseData::ProcDone {
                    id,
                    success: false,
                    code: Some(123),
                }),
            ))
            .await
            .unwrap();

        // Finally, verify that we complete and get the expected results
        assert_eq!(
            proc_wait_task.await.unwrap().unwrap(),
            RemoteStatus {
                success: false,
                code: Some(123)
            }
        );
    }

    #[test(tokio::test)]
    async fn receiving_done_response_should_result_in_output_returning_exit_information() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteCommand::new()
                .spawn(session.clone_channel(), String::from("cmd arg"))
                .await
        });

        // Wait until we get the request from the session
        let req: Request<DistantMsg<DistantRequestData>> =
            transport.read_frame_as().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .write_frame_for(&Response::new(
                req.id.clone(),
                DistantMsg::Single(DistantResponseData::ProcSpawned { id }),
            ))
            .await
            .unwrap();

        // Receive the process and then spawn a task for it to complete
        let proc = spawn_task.await.unwrap().unwrap();
        let proc_output_task = tokio::spawn(proc.output());

        // Send some stdout
        transport
            .write_frame_for(&Response::new(
                req.id.clone(),
                DistantMsg::Single(DistantResponseData::ProcStdout {
                    id,
                    data: b"some out".to_vec(),
                }),
            ))
            .await
            .unwrap();

        // Send some stderr
        transport
            .write_frame_for(&Response::new(
                req.id.clone(),
                DistantMsg::Single(DistantResponseData::ProcStderr {
                    id,
                    data: b"some err".to_vec(),
                }),
            ))
            .await
            .unwrap();

        // Send a process completion response to pass along exit status and conclude wait
        transport
            .write_frame_for(&Response::new(
                req.id,
                DistantMsg::Single(DistantResponseData::ProcDone {
                    id,
                    success: false,
                    code: Some(123),
                }),
            ))
            .await
            .unwrap();

        // Finally, verify that we complete and get the expected results
        assert_eq!(
            proc_output_task.await.unwrap().unwrap(),
            RemoteOutput {
                success: false,
                code: Some(123),
                stdout: b"some out".to_vec(),
                stderr: b"some err".to_vec(),
            }
        );
    }
}
