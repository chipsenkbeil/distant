use super::{RemoteProcess, RemoteProcessError, RemoteStderr, RemoteStdin, RemoteStdout};
use crate::{client::SessionChannel, data::PtySize};
use futures::stream::{Stream, StreamExt};
use std::{
    io::{self, Cursor, Read},
    ops::{Deref, DerefMut},
};
use tokio::{
    sync::mpsc::{self, error::TryRecvError},
    task::JoinHandle,
};

mod data;
pub use data::*;

/// Represents an LSP server process on a remote machine
#[derive(Debug)]
pub struct RemoteLspProcess {
    inner: RemoteProcess,
    pub stdin: Option<RemoteLspStdin>,
    pub stdout: Option<RemoteLspStdout>,
    pub stderr: Option<RemoteLspStderr>,
}

impl RemoteLspProcess {
    /// Spawns the specified process on the remote machine using the given session, treating
    /// the process like an LSP server
    pub async fn spawn<T>(
        channel: SessionChannel<T>,
        cmd: impl Into<String>,
        args: Vec<String>,
        persist: bool,
        pty: Option<PtySize>,
    ) -> Result<Self, RemoteProcessError> {
        let mut inner = RemoteProcess::spawn(channel, cmd, args, persist, pty).await?;
        let stdin = inner.stdin.take().map(RemoteLspStdin::new);
        let stdout = inner.stdout.take().map(RemoteLspStdout::new);
        let stderr = inner.stderr.take().map(RemoteLspStderr::new);

        Ok(RemoteLspProcess {
            inner,
            stdin,
            stdout,
            stderr,
        })
    }

    /// Waits for the process to terminate, returning the success status and an optional exit code
    pub async fn wait(self) -> Result<(bool, Option<i32>), RemoteProcessError> {
        self.inner.wait().await
    }
}

impl Deref for RemoteLspProcess {
    type Target = RemoteProcess;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for RemoteLspProcess {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

/// A handle to a remote LSP process' standard input (stdin)
#[derive(Debug)]
pub struct RemoteLspStdin {
    inner: RemoteStdin,
    buf: Option<Vec<u8>>,
}

impl RemoteLspStdin {
    pub fn new(inner: RemoteStdin) -> Self {
        Self { inner, buf: None }
    }

    /// Tries to write data to the stdin of a specific remote process
    pub fn try_write(&mut self, data: &[u8]) -> io::Result<()> {
        let queue = self.update_and_read_messages(data)?;

        // Process and then send out each LSP message in our queue
        for mut data in queue {
            // Convert distant:// to file://
            data.mut_content().convert_distant_scheme_to_local();
            data.refresh_content_length();
            self.inner.try_write_str(data.to_string())?;
        }

        Ok(())
    }

    pub fn try_write_str(&mut self, data: &str) -> io::Result<()> {
        self.try_write(data.as_bytes())
    }

    /// Writes data to the stdin of a specific remote process
    pub async fn write(&mut self, data: &[u8]) -> io::Result<()> {
        let queue = self.update_and_read_messages(data)?;

        // Process and then send out each LSP message in our queue
        for mut data in queue {
            // Convert distant:// to file://
            data.mut_content().convert_distant_scheme_to_local();
            data.refresh_content_length();
            self.inner.write_str(data.to_string()).await?;
        }

        Ok(())
    }

    pub async fn write_str(&mut self, data: &str) -> io::Result<()> {
        self.write(data.as_bytes()).await
    }

    fn update_and_read_messages(&mut self, data: &[u8]) -> io::Result<Vec<LspData>> {
        // Create or insert into our buffer
        match &mut self.buf {
            Some(buf) => buf.extend(data),
            None => self.buf = Some(data.to_vec()),
        }

        // Read LSP messages from our internal buffer
        let buf = self.buf.take().unwrap();
        match read_lsp_messages(&buf) {
            // If we succeed, update buf with our remainder and return messages
            Ok((remainder, queue)) => {
                self.buf = remainder;
                Ok(queue)
            }

            // Otherwise, if failed, reset buf back to what it was
            Err(x) => {
                self.buf = Some(buf);
                Err(x)
            }
        }
    }
}

/// A handle to a remote LSP process' standard output (stdout)
#[derive(Debug)]
pub struct RemoteLspStdout {
    read_task: JoinHandle<()>,
    rx: mpsc::Receiver<io::Result<Vec<u8>>>,
}

impl RemoteLspStdout {
    pub fn new(inner: RemoteStdout) -> Self {
        let (read_task, rx) = spawn_read_task(Box::pin(futures::stream::unfold(
            inner,
            |mut inner| async move {
                match inner.read().await {
                    Ok(res) => Some((res, inner)),
                    Err(_) => None,
                }
            },
        )));

        Self { read_task, rx }
    }

    /// Tries to read a complete LSP message over stdout, returning `None` if no complete message
    /// is available
    pub fn try_read(&mut self) -> io::Result<Option<Vec<u8>>> {
        match self.rx.try_recv() {
            Ok(Ok(data)) => Ok(Some(data)),
            Ok(Err(x)) => Err(x),
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

    /// Reads a complete LSP message over stdout
    pub async fn read(&mut self) -> io::Result<Vec<u8>> {
        self.rx
            .recv()
            .await
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?
    }

    /// Same as `read`, but returns a string
    pub async fn read_string(&mut self) -> io::Result<String> {
        self.read().await.and_then(|data| {
            String::from_utf8(data).map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))
        })
    }
}

impl Drop for RemoteLspStdout {
    fn drop(&mut self) {
        self.read_task.abort();
        self.rx.close();
    }
}

/// A handle to a remote LSP process' stderr
#[derive(Debug)]
pub struct RemoteLspStderr {
    read_task: JoinHandle<()>,
    rx: mpsc::Receiver<io::Result<Vec<u8>>>,
}

impl RemoteLspStderr {
    pub fn new(inner: RemoteStderr) -> Self {
        let (read_task, rx) = spawn_read_task(Box::pin(futures::stream::unfold(
            inner,
            |mut inner| async move {
                match inner.read().await {
                    Ok(res) => Some((res, inner)),
                    Err(_) => None,
                }
            },
        )));

        Self { read_task, rx }
    }

    /// Tries to read a complete LSP message over stderr, returning `None` if no complete message
    /// is available
    pub fn try_read(&mut self) -> io::Result<Option<Vec<u8>>> {
        match self.rx.try_recv() {
            Ok(Ok(data)) => Ok(Some(data)),
            Ok(Err(x)) => Err(x),
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

    /// Reads a complete LSP message over stderr
    pub async fn read(&mut self) -> io::Result<Vec<u8>> {
        self.rx
            .recv()
            .await
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))?
    }

    /// Same as `read`, but returns a string
    pub async fn read_string(&mut self) -> io::Result<String> {
        self.read().await.and_then(|data| {
            String::from_utf8(data).map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))
        })
    }
}

impl Drop for RemoteLspStderr {
    fn drop(&mut self) {
        self.read_task.abort();
        self.rx.close();
    }
}

fn spawn_read_task<S>(mut stream: S) -> (JoinHandle<()>, mpsc::Receiver<io::Result<Vec<u8>>>)
where
    S: Stream<Item = Vec<u8>> + Send + Unpin + 'static,
{
    let (tx, rx) = mpsc::channel::<io::Result<Vec<u8>>>(1);
    let read_task = tokio::spawn(async move {
        let mut task_buf: Option<Vec<u8>> = None;

        while let Some(data) = stream.next().await {
            // Create or insert into our buffer
            match &mut task_buf {
                Some(buf) => buf.extend(data),
                None => task_buf = Some(data),
            }

            // Read LSP messages from our internal buffer
            let buf = task_buf.take().unwrap();
            let (remainder, queue) = match read_lsp_messages(&buf) {
                Ok(x) => x,
                Err(x) => {
                    let _ = tx.send(Err(x)).await;
                    break;
                }
            };
            task_buf = remainder;

            // Process and then add each LSP message as output
            if !queue.is_empty() {
                let mut out = Vec::new();
                for mut data in queue {
                    // Convert file:// to distant://
                    data.mut_content().convert_local_scheme_to_distant();
                    data.refresh_content_length();
                    out.extend(data.to_bytes());
                }
                if tx.send(Ok(out)).await.is_err() {
                    break;
                }
            }
        }
    });

    (read_task, rx)
}

fn read_lsp_messages(input: &[u8]) -> io::Result<(Option<Vec<u8>>, Vec<LspData>)> {
    let mut queue = Vec::new();

    // Continue to read complete messages from the input until we either fail to parse or we reach
    // end of input, resetting cursor position back to last successful parse as otherwise the
    // cursor may have moved partially from lsp successfully reading the start of a message
    let mut cursor = Cursor::new(input);
    let mut pos = 0;
    while let Ok(data) = LspData::from_buf_reader(&mut cursor) {
        queue.push(data);
        pos = cursor.position();
    }
    cursor.set_position(pos);

    // Keep remainder of bytes not processed as LSP message in buffer
    let remainder = if (cursor.position() as usize) < cursor.get_ref().len() {
        let mut buf = Vec::new();
        cursor.read_to_end(&mut buf)?;
        Some(buf)
    } else {
        None
    };

    Ok((remainder, queue))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        client::Session,
        data::{DistantRequestData, DistantResponseData, Request, Response},
    };
    use distant_net::{InmemoryStream, PlainCodec, Transport};
    use std::{future::Future, time::Duration};

    /// Timeout used with timeout function
    const TIMEOUT: Duration = Duration::from_millis(50);

    // Configures an lsp process with a means to send & receive data from outside
    async fn spawn_lsp_process() -> (Transport<InmemoryStream, PlainCodec>, RemoteLspProcess) {
        let (mut t1, t2) = Transport::make_pair();
        let session = Session::initialize(t2).unwrap();
        let spawn_task = tokio::spawn(async move {
            RemoteLspProcess::spawn(
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
        let req = t1.receive::<Request>().await.unwrap().unwrap();

        // Send back a response through the session
        t1.send(Response::new(
            "test-tenant",
            req.id,
            vec![DistantResponseData::ProcSpawned { id: rand::random() }],
        ))
        .await
        .unwrap();

        // Wait for the process to be ready
        let proc = spawn_task.await.unwrap().unwrap();
        (t1, proc)
    }

    fn make_lsp_msg<T>(value: T) -> Vec<u8>
    where
        T: serde::Serialize,
    {
        let content = serde_json::to_string_pretty(&value).unwrap();
        format!("Content-Length: {}\r\n\r\n{}", content.len(), content).into_bytes()
    }

    async fn timeout<F, R>(duration: Duration, f: F) -> io::Result<R>
    where
        F: Future<Output = R>,
    {
        tokio::select! {
            res = f => {
                Ok(res)
            }
            _ = tokio::time::sleep(duration) => {
                Err(io::Error::from(io::ErrorKind::TimedOut))
            }
        }
    }

    #[tokio::test]
    async fn stdin_write_should_only_send_out_complete_lsp_messages() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        proc.stdin
            .as_mut()
            .unwrap()
            .write(&make_lsp_msg(serde_json::json!({
                "field1": "a",
                "field2": "b",
            })))
            .await
            .unwrap();

        // Validate that the outgoing req is a complete LSP message
        let req = transport.receive::<Request>().await.unwrap().unwrap();
        assert_eq!(req.payload.len(), 1, "Unexpected payload size");
        match &req.payload[0] {
            DistantRequestData::ProcStdin { data, .. } => {
                assert_eq!(
                    data,
                    &make_lsp_msg(serde_json::json!({
                        "field1": "a",
                        "field2": "b",
                    }))
                );
            }
            x => panic!("Unexpected request: {:?}", x),
        }
    }

    #[tokio::test]
    async fn stdin_write_should_support_buffering_output_until_a_complete_lsp_message_is_composed()
    {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        let msg = make_lsp_msg(serde_json::json!({
            "field1": "a",
            "field2": "b",
        }));
        let (msg_a, msg_b) = msg.split_at(msg.len() / 2);

        // Write part of the message that isn't finished
        proc.stdin.as_mut().unwrap().write(msg_a).await.unwrap();

        // Verify that nothing has been sent out yet
        // NOTE: Yield to ensure that data would be waiting at the transport if it was sent
        tokio::task::yield_now().await;
        let result = timeout(TIMEOUT, transport.receive::<Request>()).await;
        assert!(result.is_err(), "Unexpectedly got data: {:?}", result);

        // Write remainder of message
        proc.stdin.as_mut().unwrap().write(msg_b).await.unwrap();

        // Validate that the outgoing req is a complete LSP message
        let req = transport.receive::<Request>().await.unwrap().unwrap();
        assert_eq!(req.payload.len(), 1, "Unexpected payload size");
        match &req.payload[0] {
            DistantRequestData::ProcStdin { data, .. } => {
                assert_eq!(
                    data,
                    &make_lsp_msg(serde_json::json!({
                        "field1": "a",
                        "field2": "b",
                    }))
                );
            }
            x => panic!("Unexpected request: {:?}", x),
        }
    }

    #[tokio::test]
    async fn stdin_write_should_only_consume_a_complete_lsp_message_even_if_more_is_written() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        let msg = make_lsp_msg(serde_json::json!({
            "field1": "a",
            "field2": "b",
        }));

        let extra = "Content-Length: 123";

        // Write a full message plus some extra
        proc.stdin
            .as_mut()
            .unwrap()
            .write_str(&format!("{}{}", String::from_utf8(msg).unwrap(), extra))
            .await
            .unwrap();

        // Validate that the outgoing req is a complete LSP message
        let req = transport.receive::<Request>().await.unwrap().unwrap();
        assert_eq!(req.payload.len(), 1, "Unexpected payload size");
        match &req.payload[0] {
            DistantRequestData::ProcStdin { data, .. } => {
                assert_eq!(
                    data,
                    &make_lsp_msg(serde_json::json!({
                        "field1": "a",
                        "field2": "b",
                    }))
                );
            }
            x => panic!("Unexpected request: {:?}", x),
        }

        // Also validate that the internal buffer still contains the extra
        assert_eq!(
            String::from_utf8(proc.stdin.unwrap().buf.unwrap()).unwrap(),
            extra,
            "Extra was not retained"
        );
    }

    #[tokio::test]
    async fn stdin_write_should_support_sending_out_multiple_lsp_messages_if_all_received_at_once()
    {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        let msg_1 = make_lsp_msg(serde_json::json!({
            "field1": "a",
            "field2": "b",
        }));
        let msg_2 = make_lsp_msg(serde_json::json!({
            "field1": "c",
            "field2": "d",
        }));

        // Write two full messages at once
        proc.stdin
            .as_mut()
            .unwrap()
            .write_str(&format!(
                "{}{}",
                String::from_utf8(msg_1).unwrap(),
                String::from_utf8(msg_2).unwrap()
            ))
            .await
            .unwrap();

        // Validate that the first outgoing req is a complete LSP message matching first
        let req = transport.receive::<Request>().await.unwrap().unwrap();
        assert_eq!(req.payload.len(), 1, "Unexpected payload size");
        match &req.payload[0] {
            DistantRequestData::ProcStdin { data, .. } => {
                assert_eq!(
                    data,
                    &make_lsp_msg(serde_json::json!({
                        "field1": "a",
                        "field2": "b",
                    }))
                );
            }
            x => panic!("Unexpected request: {:?}", x),
        }

        // Validate that the second outgoing req is a complete LSP message matching second
        let req = transport.receive::<Request>().await.unwrap().unwrap();
        assert_eq!(req.payload.len(), 1, "Unexpected payload size");
        match &req.payload[0] {
            DistantRequestData::ProcStdin { data, .. } => {
                assert_eq!(
                    data,
                    &make_lsp_msg(serde_json::json!({
                        "field1": "c",
                        "field2": "d",
                    }))
                );
            }
            x => panic!("Unexpected request: {:?}", x),
        }
    }

    #[tokio::test]
    async fn stdin_write_should_convert_content_with_distant_scheme_to_file_scheme() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        proc.stdin
            .as_mut()
            .unwrap()
            .write(&make_lsp_msg(serde_json::json!({
                "field1": "distant://some/path",
                "field2": "file://other/path",
            })))
            .await
            .unwrap();

        // Validate that the outgoing req is a complete LSP message
        let req = transport.receive::<Request>().await.unwrap().unwrap();
        assert_eq!(req.payload.len(), 1, "Unexpected payload size");
        match &req.payload[0] {
            DistantRequestData::ProcStdin { data, .. } => {
                // Verify the contents AND headers are as expected; in this case,
                // this will also ensure that the Content-Length is adjusted
                // when the distant scheme was changed to file
                assert_eq!(
                    data,
                    &make_lsp_msg(serde_json::json!({
                        "field1": "file://some/path",
                        "field2": "file://other/path",
                    }))
                );
            }
            x => panic!("Unexpected request: {:?}", x),
        }
    }

    #[tokio::test]
    async fn stdout_read_should_yield_lsp_messages_as_strings() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        // Send complete LSP message as stdout to process
        transport
            .send(Response::new(
                "test-tenant",
                proc.origin_id(),
                vec![DistantResponseData::ProcStdout {
                    id: proc.id(),
                    data: make_lsp_msg(serde_json::json!({
                        "field1": "a",
                        "field2": "b",
                    })),
                }],
            ))
            .await
            .unwrap();

        // Receive complete message as stdout from process
        let out = proc.stdout.as_mut().unwrap().read().await.unwrap();
        assert_eq!(
            out,
            make_lsp_msg(serde_json::json!({
                "field1": "a",
                "field2": "b",
            }))
        );
    }

    #[tokio::test]
    async fn stdout_read_should_only_yield_complete_lsp_messages() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        let msg = make_lsp_msg(serde_json::json!({
            "field1": "a",
            "field2": "b",
        }));
        let (msg_a, msg_b) = msg.split_at(msg.len() / 2);

        // Send half of LSP message over stdout
        transport
            .send(Response::new(
                "test-tenant",
                proc.origin_id(),
                vec![DistantResponseData::ProcStdout {
                    id: proc.id(),
                    data: msg_a.to_vec(),
                }],
            ))
            .await
            .unwrap();

        // Verify that remote process has not received a complete message yet
        // NOTE: Yield to ensure that data would be waiting at the transport if it was sent
        tokio::task::yield_now().await;
        let result = timeout(TIMEOUT, proc.stdout.as_mut().unwrap().read()).await;
        assert!(result.is_err(), "Unexpectedly got data: {:?}", result);

        // Send other half of LSP message over stdout
        transport
            .send(Response::new(
                "test-tenant",
                proc.origin_id(),
                vec![DistantResponseData::ProcStdout {
                    id: proc.id(),
                    data: msg_b.to_vec(),
                }],
            ))
            .await
            .unwrap();

        // Receive complete message as stdout from process
        let out = proc.stdout.as_mut().unwrap().read().await.unwrap();
        assert_eq!(
            out,
            make_lsp_msg(serde_json::json!({
                "field1": "a",
                "field2": "b",
            }))
        );
    }

    #[tokio::test]
    async fn stdout_read_should_only_consume_a_complete_lsp_message_even_if_more_output_is_available(
    ) {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        let msg = make_lsp_msg(serde_json::json!({
            "field1": "a",
            "field2": "b",
        }));
        let extra = "some extra content";

        // Send complete LSP message as stdout to process
        transport
            .send(Response::new(
                "test-tenant",
                proc.origin_id(),
                vec![DistantResponseData::ProcStdout {
                    id: proc.id(),
                    data: format!("{}{}", String::from_utf8(msg).unwrap(), extra).into_bytes(),
                }],
            ))
            .await
            .unwrap();

        // Receive complete message as stdout from process
        let out = proc.stdout.as_mut().unwrap().read().await.unwrap();
        assert_eq!(
            out,
            make_lsp_msg(serde_json::json!({
                "field1": "a",
                "field2": "b",
            }))
        );

        // Verify nothing else was sent
        let result = timeout(TIMEOUT, proc.stdout.as_mut().unwrap().read()).await;
        assert!(
            result.is_err(),
            "Unexpected extra content received on stdout"
        );
    }

    #[tokio::test]
    async fn stdout_read_should_support_yielding_multiple_lsp_messages_if_all_received_at_once() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        let msg_1 = make_lsp_msg(serde_json::json!({
            "field1": "a",
            "field2": "b",
        }));
        let msg_2 = make_lsp_msg(serde_json::json!({
            "field1": "c",
            "field2": "d",
        }));

        // Send complete LSP message as stdout to process
        transport
            .send(Response::new(
                "test-tenant",
                proc.origin_id(),
                vec![DistantResponseData::ProcStdout {
                    id: proc.id(),
                    data: format!(
                        "{}{}",
                        String::from_utf8(msg_1).unwrap(),
                        String::from_utf8(msg_2).unwrap()
                    )
                    .into_bytes(),
                }],
            ))
            .await
            .unwrap();

        // Should send both messages back together as a single string
        let out = proc.stdout.as_mut().unwrap().read().await.unwrap();
        assert_eq!(
            out,
            format!(
                "{}{}",
                String::from_utf8(make_lsp_msg(serde_json::json!({
                    "field1": "a",
                    "field2": "b",
                })))
                .unwrap(),
                String::from_utf8(make_lsp_msg(serde_json::json!({
                    "field1": "c",
                    "field2": "d",
                })))
                .unwrap()
            )
            .into_bytes()
        );
    }

    #[tokio::test]
    async fn stdout_read_should_convert_content_with_file_scheme_to_distant_scheme() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        // Send complete LSP message as stdout to process
        transport
            .send(Response::new(
                "test-tenant",
                proc.origin_id(),
                vec![DistantResponseData::ProcStdout {
                    id: proc.id(),
                    data: make_lsp_msg(serde_json::json!({
                        "field1": "distant://some/path",
                        "field2": "file://other/path",
                    })),
                }],
            ))
            .await
            .unwrap();

        // Receive complete message as stdout from process
        let out = proc.stdout.as_mut().unwrap().read().await.unwrap();
        assert_eq!(
            out,
            make_lsp_msg(serde_json::json!({
                "field1": "distant://some/path",
                "field2": "distant://other/path",
            }))
        );
    }

    #[tokio::test]
    async fn stderr_read_should_yield_lsp_messages_as_strings() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        // Send complete LSP message as stderr to process
        transport
            .send(Response::new(
                "test-tenant",
                proc.origin_id(),
                vec![DistantResponseData::ProcStderr {
                    id: proc.id(),
                    data: make_lsp_msg(serde_json::json!({
                        "field1": "a",
                        "field2": "b",
                    })),
                }],
            ))
            .await
            .unwrap();

        // Receive complete message as stderr from process
        let err = proc.stderr.as_mut().unwrap().read().await.unwrap();
        assert_eq!(
            err,
            make_lsp_msg(serde_json::json!({
                "field1": "a",
                "field2": "b",
            }))
        );
    }

    #[tokio::test]
    async fn stderr_read_should_only_yield_complete_lsp_messages() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        let msg = make_lsp_msg(serde_json::json!({
            "field1": "a",
            "field2": "b",
        }));
        let (msg_a, msg_b) = msg.split_at(msg.len() / 2);

        // Send half of LSP message over stderr
        transport
            .send(Response::new(
                "test-tenant",
                proc.origin_id(),
                vec![DistantResponseData::ProcStderr {
                    id: proc.id(),
                    data: msg_a.to_vec(),
                }],
            ))
            .await
            .unwrap();

        // Verify that remote process has not received a complete message yet
        // NOTE: Yield to ensure that data would be waiting at the transport if it was sent
        tokio::task::yield_now().await;
        let result = timeout(TIMEOUT, proc.stderr.as_mut().unwrap().read()).await;
        assert!(result.is_err(), "Unexpectedly got data: {:?}", result);

        // Send other half of LSP message over stderr
        transport
            .send(Response::new(
                "test-tenant",
                proc.origin_id(),
                vec![DistantResponseData::ProcStderr {
                    id: proc.id(),
                    data: msg_b.to_vec(),
                }],
            ))
            .await
            .unwrap();

        // Receive complete message as stderr from process
        let err = proc.stderr.as_mut().unwrap().read().await.unwrap();
        assert_eq!(
            err,
            make_lsp_msg(serde_json::json!({
                "field1": "a",
                "field2": "b",
            }))
        );
    }

    #[tokio::test]
    async fn stderr_read_should_only_consume_a_complete_lsp_message_even_if_more_errput_is_available(
    ) {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        let msg = make_lsp_msg(serde_json::json!({
            "field1": "a",
            "field2": "b",
        }));
        let extra = "some extra content";

        // Send complete LSP message as stderr to process
        transport
            .send(Response::new(
                "test-tenant",
                proc.origin_id(),
                vec![DistantResponseData::ProcStderr {
                    id: proc.id(),
                    data: format!("{}{}", String::from_utf8(msg).unwrap(), extra).into_bytes(),
                }],
            ))
            .await
            .unwrap();

        // Receive complete message as stderr from process
        let err = proc.stderr.as_mut().unwrap().read().await.unwrap();
        assert_eq!(
            err,
            make_lsp_msg(serde_json::json!({
                "field1": "a",
                "field2": "b",
            }))
        );

        // Verify nothing else was sent
        let result = timeout(TIMEOUT, proc.stderr.as_mut().unwrap().read()).await;
        assert!(
            result.is_err(),
            "Unexpected extra content received on stderr"
        );
    }

    #[tokio::test]
    async fn stderr_read_should_support_yielding_multiple_lsp_messages_if_all_received_at_once() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        let msg_1 = make_lsp_msg(serde_json::json!({
            "field1": "a",
            "field2": "b",
        }));
        let msg_2 = make_lsp_msg(serde_json::json!({
            "field1": "c",
            "field2": "d",
        }));

        // Send complete LSP message as stderr to process
        transport
            .send(Response::new(
                "test-tenant",
                proc.origin_id(),
                vec![DistantResponseData::ProcStderr {
                    id: proc.id(),
                    data: format!(
                        "{}{}",
                        String::from_utf8(msg_1).unwrap(),
                        String::from_utf8(msg_2).unwrap()
                    )
                    .into_bytes(),
                }],
            ))
            .await
            .unwrap();

        // Should send both messages back together as a single string
        let err = proc.stderr.as_mut().unwrap().read().await.unwrap();
        assert_eq!(
            err,
            format!(
                "{}{}",
                String::from_utf8(make_lsp_msg(serde_json::json!({
                    "field1": "a",
                    "field2": "b",
                })))
                .unwrap(),
                String::from_utf8(make_lsp_msg(serde_json::json!({
                    "field1": "c",
                    "field2": "d",
                })))
                .unwrap()
            )
            .into_bytes()
        );
    }

    #[tokio::test]
    async fn stderr_read_should_convert_content_with_file_scheme_to_distant_scheme() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        // Send complete LSP message as stderr to process
        transport
            .send(Response::new(
                "test-tenant",
                proc.origin_id(),
                vec![DistantResponseData::ProcStderr {
                    id: proc.id(),
                    data: make_lsp_msg(serde_json::json!({
                        "field1": "distant://some/path",
                        "field2": "file://other/path",
                    })),
                }],
            ))
            .await
            .unwrap();

        // Receive complete message as stderr from process
        let err = proc.stderr.as_mut().unwrap().read().await.unwrap();
        assert_eq!(
            err,
            make_lsp_msg(serde_json::json!({
                "field1": "distant://some/path",
                "field2": "distant://other/path",
            }))
        );
    }
}
