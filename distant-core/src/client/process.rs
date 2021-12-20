use crate::{
    client::{Mailbox, SessionChannel},
    constants::CLIENT_PIPE_CAPACITY,
    data::{PtySize, Request, RequestData, ResponseData},
    net::TransportError,
};
use derive_more::{Display, Error, From};
use log::*;
use std::sync::Arc;
use tokio::{
    io,
    sync::{
        mpsc::{
            self,
            error::{TryRecvError, TrySendError},
        },
        RwLock,
    },
    task::{JoinError, JoinHandle},
};

type StatusResult = Result<(bool, Option<i32>), RemoteProcessError>;

#[derive(Debug, Display, Error, From)]
pub enum RemoteProcessError {
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
#[derive(Debug)]
pub struct RemoteProcess {
    /// Id of the process
    id: usize,

    /// Id used to map back to mailbox
    origin_id: usize,

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

    /// Sender for kill events
    kill: mpsc::Sender<()>,

    /// Task that waits for the process to complete
    wait_task: JoinHandle<()>,

    /// Handles the success and exit code for a completed process
    status: Arc<RwLock<Option<StatusResult>>>,
}

impl RemoteProcess {
    /// Spawns the specified process on the remote machine using the given session
    pub async fn spawn(
        tenant: impl Into<String>,
        mut channel: SessionChannel,
        cmd: impl Into<String>,
        args: Vec<String>,
        detached: bool,
        pty: Option<PtySize>,
    ) -> Result<Self, RemoteProcessError> {
        let tenant = tenant.into();
        let cmd = cmd.into();

        // Submit our run request and get back a mailbox for responses
        let mut mailbox = channel
            .mail(Request::new(
                tenant.as_str(),
                vec![RequestData::ProcSpawn {
                    cmd,
                    args,
                    detached,
                    pty,
                }],
            ))
            .await?;

        // Wait until we get the first response, and get id from proc started
        let (id, origin_id) = match mailbox.next().await {
            Some(res) if res.payload.len() != 1 => {
                return Err(RemoteProcessError::TransportError(TransportError::IoError(
                    io::Error::new(io::ErrorKind::InvalidData, "Got wrong payload size"),
                )));
            }
            Some(res) => {
                let origin_id = res.origin_id;
                match res.payload.into_iter().next().unwrap() {
                    ResponseData::ProcSpawned { id } => (id, origin_id),
                    ResponseData::Error(x) => {
                        return Err(RemoteProcessError::TransportError(TransportError::IoError(
                            x.into(),
                        )))
                    }
                    x => {
                        return Err(RemoteProcessError::TransportError(TransportError::IoError(
                            io::Error::new(
                                io::ErrorKind::InvalidData,
                                format!("Got response type of {}", x.as_ref()),
                            ),
                        )))
                    }
                }
            }
            None => {
                return Err(RemoteProcessError::TransportError(TransportError::IoError(
                    io::Error::from(io::ErrorKind::ConnectionAborted),
                )))
            }
        };

        // Create channels for our stdin/stdout/stderr
        let (stdin_tx, stdin_rx) = mpsc::channel(CLIENT_PIPE_CAPACITY);
        let (stdout_tx, stdout_rx) = mpsc::channel(CLIENT_PIPE_CAPACITY);
        let (stderr_tx, stderr_rx) = mpsc::channel(CLIENT_PIPE_CAPACITY);

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
                res = process_outgoing_requests(tenant, id, channel, stdin_rx, kill_rx) => {
                    res
                }
            }
        });

        let status = Arc::new(RwLock::new(None));
        let status_2 = Arc::clone(&status);
        let wait_task = tokio::spawn(async move {
            let res = match tokio::try_join!(req_task, res_task) {
                Ok((_, res)) => res,
                Err(x) => Err(RemoteProcessError::from(x)),
            };
            status_2.write().await.replace(res);
        });

        Ok(Self {
            id,
            origin_id,
            abort_req_task_tx,
            abort_res_task_tx,
            stdin: Some(RemoteStdin(stdin_tx)),
            stdout: Some(RemoteStdout(stdout_rx)),
            stderr: Some(RemoteStderr(stderr_rx)),
            kill: kill_tx,
            wait_task,
            status,
        })
    }

    /// Returns the id of the running process
    pub fn id(&self) -> usize {
        self.id
    }

    /// Returns the id of the request that spawned this process
    pub fn origin_id(&self) -> usize {
        self.origin_id
    }

    /// Checks if the process has completed, returning the exit status if it has, without
    /// consuming the process itself. Note that this does not include join errors that can
    /// occur when aborting and instead converts any error to a status of false. To acquire
    /// the actual error, you must call `wait`
    pub async fn status(&self) -> Option<(bool, Option<i32>)> {
        self.status.read().await.as_ref().map(|x| match x {
            Ok((success, exit_code)) => (*success, *exit_code),
            Err(_) => (false, None),
        })
    }

    /// Waits for the process to terminate, returning the success status and an optional exit code
    pub async fn wait(self) -> Result<(bool, Option<i32>), RemoteProcessError> {
        // Wait for the process to complete before we try to get the status
        let _ = self.wait_task.await;

        // NOTE: If we haven't received an exit status, this lines up with the UnexpectedEof error
        self.status
            .write()
            .await
            .take()
            .unwrap_or(Err(RemoteProcessError::UnexpectedEof))
    }

    /// Aborts the process by forcing its response task to shutdown, which means that a call
    /// to `wait` will return an error. Note that this does **not** send a kill request, so if
    /// you want to be nice you should send the request before aborting.
    pub fn abort(&self) {
        let _ = self.abort_req_task_tx.try_send(());
        let _ = self.abort_res_task_tx.try_send(());
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
#[derive(Debug)]
pub struct RemoteStdin(mpsc::Sender<Vec<u8>>);

impl RemoteStdin {
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
    tenant: String,
    id: usize,
    mut channel: SessionChannel,
    mut stdin_rx: mpsc::Receiver<Vec<u8>>,
    mut kill_rx: mpsc::Receiver<()>,
) -> Result<(), RemoteProcessError> {
    let result = loop {
        tokio::select! {
            data = stdin_rx.recv() => {
                match data {
                    Some(data) => channel.fire(
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
                    channel.fire(Request::new(
                        tenant.as_str(),
                        vec![RequestData::ProcKill { id }],
                    )).await?;
                    break Ok(());
                } else {
                    break Err(RemoteProcessError::ChannelDead);
                }
            }
        }
    };

    trace!("Process outgoing channel closed");
    result
}

/// Helper function that loops, processing incoming stdout & stderr requests from a remote process
async fn process_incoming_responses(
    proc_id: usize,
    mut mailbox: Mailbox,
    stdout_tx: mpsc::Sender<Vec<u8>>,
    stderr_tx: mpsc::Sender<Vec<u8>>,
    kill_tx: mpsc::Sender<()>,
) -> Result<(bool, Option<i32>), RemoteProcessError> {
    while let Some(res) = mailbox.next().await {
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
                    let _ = stdout_tx.send(data).await;
                }
                ResponseData::ProcStderr { id, data } if id == proc_id => {
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
    Err(RemoteProcessError::UnexpectedEof)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::Session,
        data::{Error, ErrorKind, Response},
        net::{InmemoryStream, PlainCodec, Transport},
    };
    use std::time::Duration;

    fn make_session() -> (Transport<InmemoryStream, PlainCodec>, Session) {
        let (t1, t2) = Transport::make_pair();
        (t1, Session::initialize(t2).unwrap())
    }

    #[tokio::test]
    async fn spawn_should_return_invalid_data_if_payload_size_unexpected() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteProcess::spawn(
                String::from("test-tenant"),
                session.clone_channel(),
                String::from("cmd"),
                vec![String::from("arg")],
                false,
                None,
            )
            .await
        });

        // Wait until we get the request from the session
        let req = transport.receive::<Request>().await.unwrap().unwrap();

        // Send back a response through the session
        transport
            .send(Response::new("test-tenant", req.id, Vec::new()))
            .await
            .unwrap();

        // Get the spawn result and verify
        let result = spawn_task.await.unwrap();
        assert!(
            matches!(
                &result,
                Err(RemoteProcessError::TransportError(TransportError::IoError(x)))
                    if x.kind() == io::ErrorKind::InvalidData
            ),
            "Unexpected result: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn spawn_should_return_invalid_data_if_did_not_get_a_indicator_that_process_started() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteProcess::spawn(
                String::from("test-tenant"),
                session.clone_channel(),
                String::from("cmd"),
                vec![String::from("arg")],
                false,
                None,
            )
            .await
        });

        // Wait until we get the request from the session
        let req = transport.receive::<Request>().await.unwrap().unwrap();

        // Send back a response through the session
        transport
            .send(Response::new(
                "test-tenant",
                req.id,
                vec![ResponseData::Error(Error {
                    kind: ErrorKind::BrokenPipe,
                    description: String::from("some error"),
                })],
            ))
            .await
            .unwrap();

        // Get the spawn result and verify
        let result = spawn_task.await.unwrap();
        assert!(
            matches!(
                &result,
                Err(RemoteProcessError::TransportError(TransportError::IoError(x)))
                    if x.kind() == io::ErrorKind::BrokenPipe
            ),
            "Unexpected result: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn kill_should_return_error_if_internal_tasks_already_completed() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteProcess::spawn(
                String::from("test-tenant"),
                session.clone_channel(),
                String::from("cmd"),
                vec![String::from("arg")],
                false,
                None,
            )
            .await
        });

        // Wait until we get the request from the session
        let req = transport.receive::<Request>().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .send(Response::new(
                "test-tenant",
                req.id,
                vec![ResponseData::ProcSpawned { id }],
            ))
            .await
            .unwrap();

        // Receive the process and then abort it to make kill fail
        let mut proc = spawn_task.await.unwrap().unwrap();
        proc.abort();

        // Ensure that the other tasks are aborted before continuing
        tokio::task::yield_now().await;

        let result = proc.kill().await;
        assert!(
            matches!(result, Err(RemoteProcessError::ChannelDead)),
            "Unexpected result: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn kill_should_send_proc_kill_request_and_then_cause_stdin_forwarding_to_close() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteProcess::spawn(
                String::from("test-tenant"),
                session.clone_channel(),
                String::from("cmd"),
                vec![String::from("arg")],
                false,
                None,
            )
            .await
        });

        // Wait until we get the request from the session
        let req = transport.receive::<Request>().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .send(Response::new(
                "test-tenant",
                req.id,
                vec![ResponseData::ProcSpawned { id }],
            ))
            .await
            .unwrap();

        // Receive the process and then kill it
        let mut proc = spawn_task.await.unwrap().unwrap();
        assert!(proc.kill().await.is_ok(), "Failed to send kill request");

        // Verify the kill request was sent
        let req = transport.receive::<Request>().await.unwrap().unwrap();
        assert_eq!(
            req.payload.len(),
            1,
            "Unexpected payload length for kill request"
        );
        assert_eq!(req.payload[0], RequestData::ProcKill { id });

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

    #[tokio::test]
    async fn stdin_should_be_forwarded_from_receiver_field() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteProcess::spawn(
                String::from("test-tenant"),
                session.clone_channel(),
                String::from("cmd"),
                vec![String::from("arg")],
                false,
                None,
            )
            .await
        });

        // Wait until we get the request from the session
        let req = transport.receive::<Request>().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .send(Response::new(
                "test-tenant",
                req.id,
                vec![ResponseData::ProcSpawned { id }],
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
        match &transport
            .receive::<Request>()
            .await
            .unwrap()
            .unwrap()
            .payload[0]
        {
            RequestData::ProcStdin { id, data } => {
                assert_eq!(*id, 12345);
                assert_eq!(data, b"some input");
            }
            x => panic!("Unexpected request: {:?}", x),
        }
    }

    #[tokio::test]
    async fn stdout_should_be_forwarded_to_receiver_field() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteProcess::spawn(
                String::from("test-tenant"),
                session.clone_channel(),
                String::from("cmd"),
                vec![String::from("arg")],
                false,
                None,
            )
            .await
        });

        // Wait until we get the request from the session
        let req = transport.receive::<Request>().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .send(Response::new(
                "test-tenant",
                req.id,
                vec![ResponseData::ProcSpawned { id }],
            ))
            .await
            .unwrap();

        // Receive the process and then read stdout
        let mut proc = spawn_task.await.unwrap().unwrap();

        transport
            .send(Response::new(
                "test-tenant",
                req.id,
                vec![ResponseData::ProcStdout {
                    id,
                    data: b"some out".to_vec(),
                }],
            ))
            .await
            .unwrap();

        let out = proc.stdout.as_mut().unwrap().read().await.unwrap();
        assert_eq!(out, b"some out");
    }

    #[tokio::test]
    async fn stderr_should_be_forwarded_to_receiver_field() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteProcess::spawn(
                String::from("test-tenant"),
                session.clone_channel(),
                String::from("cmd"),
                vec![String::from("arg")],
                false,
                None,
            )
            .await
        });

        // Wait until we get the request from the session
        let req = transport.receive::<Request>().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .send(Response::new(
                "test-tenant",
                req.id,
                vec![ResponseData::ProcSpawned { id }],
            ))
            .await
            .unwrap();

        // Receive the process and then read stderr
        let mut proc = spawn_task.await.unwrap().unwrap();

        transport
            .send(Response::new(
                "test-tenant",
                req.id,
                vec![ResponseData::ProcStderr {
                    id,
                    data: b"some err".to_vec(),
                }],
            ))
            .await
            .unwrap();

        let out = proc.stderr.as_mut().unwrap().read().await.unwrap();
        assert_eq!(out, b"some err");
    }

    #[tokio::test]
    async fn status_should_return_none_if_not_done() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteProcess::spawn(
                String::from("test-tenant"),
                session.clone_channel(),
                String::from("cmd"),
                vec![String::from("arg")],
                false,
                None,
            )
            .await
        });

        // Wait until we get the request from the session
        let req = transport.receive::<Request>().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .send(Response::new(
                "test-tenant",
                req.id,
                vec![ResponseData::ProcSpawned { id }],
            ))
            .await
            .unwrap();

        // Receive the process and then check its status
        let proc = spawn_task.await.unwrap().unwrap();

        let result = proc.status().await;
        assert_eq!(result, None, "Unexpectedly got proc status: {:?}", result);
    }

    #[tokio::test]
    async fn status_should_return_false_for_success_if_internal_tasks_fail() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteProcess::spawn(
                String::from("test-tenant"),
                session.clone_channel(),
                String::from("cmd"),
                vec![String::from("arg")],
                false,
                None,
            )
            .await
        });

        // Wait until we get the request from the session
        let req = transport.receive::<Request>().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .send(Response::new(
                "test-tenant",
                req.id,
                vec![ResponseData::ProcSpawned { id }],
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
            Some((false, None)) => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[tokio::test]
    async fn status_should_return_process_status_when_done() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteProcess::spawn(
                String::from("test-tenant"),
                session.clone_channel(),
                String::from("cmd"),
                vec![String::from("arg")],
                false,
                None,
            )
            .await
        });

        // Wait until we get the request from the session
        let req = transport.receive::<Request>().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .send(Response::new(
                "test-tenant",
                req.id,
                vec![ResponseData::ProcSpawned { id }],
            ))
            .await
            .unwrap();

        // Receive the process and then spawn a task for it to complete
        let proc = spawn_task.await.unwrap().unwrap();

        // Send a process completion response to pass along exit status and conclude wait
        transport
            .send(Response::new(
                "test-tenant",
                req.id,
                vec![ResponseData::ProcDone {
                    id,
                    success: true,
                    code: Some(123),
                }],
            ))
            .await
            .unwrap();

        // Wait a bit to ensure the status gets transmitted
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Finally, verify that we complete and get the expected results
        assert_eq!(proc.status().await, Some((true, Some(123))));
    }

    #[tokio::test]
    async fn wait_should_return_error_if_internal_tasks_fail() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteProcess::spawn(
                String::from("test-tenant"),
                session.clone_channel(),
                String::from("cmd"),
                vec![String::from("arg")],
                false,
                None,
            )
            .await
        });

        // Wait until we get the request from the session
        let req = transport.receive::<Request>().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .send(Response::new(
                "test-tenant",
                req.id,
                vec![ResponseData::ProcSpawned { id }],
            ))
            .await
            .unwrap();

        // Receive the process and then abort it to make internal tasks fail
        let proc = spawn_task.await.unwrap().unwrap();
        proc.abort();

        let result = proc.wait().await;
        assert!(
            matches!(result, Err(RemoteProcessError::WaitFailed(_))),
            "Unexpected result: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn wait_should_return_error_if_connection_terminates_before_receiving_done_response() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteProcess::spawn(
                String::from("test-tenant"),
                session.clone_channel(),
                String::from("cmd"),
                vec![String::from("arg")],
                false,
                None,
            )
            .await
        });

        // Wait until we get the request from the session
        let req = transport.receive::<Request>().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .send(Response::new(
                "test-tenant",
                req.id,
                vec![ResponseData::ProcSpawned { id }],
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

        let result = proc.wait().await;
        assert!(
            matches!(result, Err(RemoteProcessError::UnexpectedEof)),
            "Unexpected result: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn receiving_done_response_should_result_in_wait_returning_exit_information() {
        let (mut transport, session) = make_session();

        // Create a task for process spawning as we need to handle the request and a response
        // in a separate async block
        let spawn_task = tokio::spawn(async move {
            RemoteProcess::spawn(
                String::from("test-tenant"),
                session.clone_channel(),
                String::from("cmd"),
                vec![String::from("arg")],
                false,
                None,
            )
            .await
        });

        // Wait until we get the request from the session
        let req = transport.receive::<Request>().await.unwrap().unwrap();

        // Send back a response through the session
        let id = 12345;
        transport
            .send(Response::new(
                "test-tenant",
                req.id,
                vec![ResponseData::ProcSpawned { id }],
            ))
            .await
            .unwrap();

        // Receive the process and then spawn a task for it to complete
        let proc = spawn_task.await.unwrap().unwrap();
        let proc_wait_task = tokio::spawn(proc.wait());

        // Send a process completion response to pass along exit status and conclude wait
        transport
            .send(Response::new(
                "test-tenant",
                req.id,
                vec![ResponseData::ProcDone {
                    id,
                    success: false,
                    code: Some(123),
                }],
            ))
            .await
            .unwrap();

        // Finally, verify that we complete and get the expected results
        assert_eq!(proc_wait_task.await.unwrap().unwrap(), (false, Some(123)));
    }
}
