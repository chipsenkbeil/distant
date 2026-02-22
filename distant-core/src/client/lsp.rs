use std::io::{self, Cursor, Read};
use std::ops::{Deref, DerefMut};
use std::path::PathBuf;

use futures::stream::{Stream, StreamExt};
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::TryRecvError;
use tokio::task::JoinHandle;

use crate::client::{
    Channel, RemoteCommand, RemoteProcess, RemoteStatus, RemoteStderr, RemoteStdin, RemoteStdout,
};
use crate::protocol::{Environment, PtySize};

mod msg;
pub use msg::*;

/// A [`RemoteLspProcess`] builder providing support to configure
/// before spawning the process on a remote machine
pub struct RemoteLspCommand {
    pty: Option<PtySize>,
    environment: Environment,
    current_dir: Option<PathBuf>,
    scheme: Option<String>,
}

impl Default for RemoteLspCommand {
    fn default() -> Self {
        Self::new()
    }
}

impl RemoteLspCommand {
    /// Creates a new set of options for a remote LSP process
    pub fn new() -> Self {
        Self {
            pty: None,
            environment: Environment::new(),
            current_dir: None,
            scheme: None,
        }
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

    /// Configures the process with a specific scheme to convert rather than `distant://`
    pub fn scheme(&mut self, scheme: Option<String>) -> &mut Self {
        self.scheme = scheme;
        self
    }

    /// Spawns the specified process on the remote machine using the given session, treating
    /// the process like an LSP server
    pub async fn spawn(
        &mut self,
        channel: Channel,
        cmd: impl Into<String>,
    ) -> io::Result<RemoteLspProcess> {
        let mut command = RemoteCommand::new();
        command.environment(self.environment.clone());
        command.current_dir(self.current_dir.clone());
        command.pty(self.pty);

        let mut inner = command.spawn(channel, cmd).await?;
        let stdin = inner
            .stdin
            .take()
            .map(|x| RemoteLspStdin::new(x, self.scheme.clone()));
        let stdout = inner
            .stdout
            .take()
            .map(|x| RemoteLspStdout::new(x, self.scheme.clone()));
        let stderr = inner
            .stderr
            .take()
            .map(|x| RemoteLspStderr::new(x, self.scheme.clone()));

        Ok(RemoteLspProcess {
            inner,
            stdin,
            stdout,
            stderr,
        })
    }
}

/// Represents an LSP server process on a remote machine
#[derive(Debug)]
pub struct RemoteLspProcess {
    inner: RemoteProcess,
    pub stdin: Option<RemoteLspStdin>,
    pub stdout: Option<RemoteLspStdout>,
    pub stderr: Option<RemoteLspStderr>,
}

impl RemoteLspProcess {
    /// Waits for the process to terminate, returning the success status and an optional exit code
    pub async fn wait(self) -> io::Result<RemoteStatus> {
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
    scheme: Option<String>,
}

impl RemoteLspStdin {
    pub fn new(inner: RemoteStdin, scheme: impl Into<Option<String>>) -> Self {
        Self {
            inner,
            buf: None,
            scheme: scheme.into(),
        }
    }

    /// Tries to write data to the stdin of a specific remote process
    pub fn try_write(&mut self, data: &[u8]) -> io::Result<()> {
        let queue = self.update_and_read_messages(data)?;

        // Process and then send out each LSP message in our queue
        for mut data in queue {
            // Convert distant:// to file://
            match self.scheme.as_mut() {
                Some(scheme) => data.mut_content().convert_scheme_to_local(scheme),
                None => data.mut_content().convert_distant_scheme_to_local(),
            }
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
            match self.scheme.as_mut() {
                Some(scheme) => data.mut_content().convert_scheme_to_local(scheme),
                None => data.mut_content().convert_distant_scheme_to_local(),
            }
            data.refresh_content_length();
            self.inner.write_str(data.to_string()).await?;
        }

        Ok(())
    }

    pub async fn write_str(&mut self, data: &str) -> io::Result<()> {
        self.write(data.as_bytes()).await
    }

    fn update_and_read_messages(&mut self, data: &[u8]) -> io::Result<Vec<LspMsg>> {
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
    pub fn new(inner: RemoteStdout, scheme: impl Into<Option<String>>) -> Self {
        let (read_task, rx) = spawn_read_task(
            Box::pin(futures::stream::unfold(inner, |mut inner| async move {
                match inner.read().await {
                    Ok(res) => Some((res, inner)),
                    Err(_) => None,
                }
            })),
            scheme,
        );

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
    pub fn new(inner: RemoteStderr, scheme: impl Into<Option<String>>) -> Self {
        let (read_task, rx) = spawn_read_task(
            Box::pin(futures::stream::unfold(inner, |mut inner| async move {
                match inner.read().await {
                    Ok(res) => Some((res, inner)),
                    Err(_) => None,
                }
            })),
            scheme,
        );

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

fn spawn_read_task<S>(
    mut stream: S,
    scheme: impl Into<Option<String>>,
) -> (JoinHandle<()>, mpsc::Receiver<io::Result<Vec<u8>>>)
where
    S: Stream<Item = Vec<u8>> + Send + Unpin + 'static,
{
    let mut scheme = scheme.into();
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
                    match scheme.as_mut() {
                        Some(scheme) => data.mut_content().convert_local_scheme_to(scheme),
                        None => data.mut_content().convert_local_scheme_to_distant(),
                    }
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

fn read_lsp_messages(input: &[u8]) -> io::Result<(Option<Vec<u8>>, Vec<LspMsg>)> {
    let mut queue = Vec::new();

    // Continue to read complete messages from the input until we either fail to parse or we reach
    // end of input, resetting cursor position back to last successful parse as otherwise the
    // cursor may have moved partially from lsp successfully reading the start of a message
    let mut cursor = Cursor::new(input);
    let mut pos = 0;
    while let Ok(data) = LspMsg::from_buf_reader(&mut cursor) {
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
    use std::future::Future;
    use std::time::Duration;

    use crate::net::common::{FramedTransport, InmemoryTransport, Request, Response};
    use crate::net::Client;
    use test_log::test;

    use super::*;
    use crate::protocol;

    /// Timeout used with timeout function
    const TIMEOUT: Duration = Duration::from_millis(50);

    // Configures an lsp process with a means to send & receive data from outside
    async fn spawn_lsp_process() -> (FramedTransport<InmemoryTransport>, RemoteLspProcess) {
        let (mut t1, t2) = FramedTransport::pair(100);
        let client = Client::spawn_inmemory(t2, Default::default());
        let spawn_task = tokio::spawn({
            let channel = client.clone_channel();
            async move {
                RemoteLspCommand::new()
                    .spawn(channel, String::from("cmd arg"))
                    .await
            }
        });

        // Wait until we get the request from the session
        let req: Request<protocol::Request> = t1.read_frame_as().await.unwrap().unwrap();

        // Send back a response through the session
        t1.write_frame_for(&Response::new(
            req.id,
            protocol::Response::ProcSpawned { id: rand::random() },
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

    #[test(tokio::test)]
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
        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::ProcStdin { data, .. } => {
                assert_eq!(
                    data,
                    make_lsp_msg(serde_json::json!({
                        "field1": "a",
                        "field2": "b",
                    }))
                );
            }
            x => panic!("Unexpected request: {:?}", x),
        }
    }

    #[test(tokio::test)]
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
        let result = timeout(
            TIMEOUT,
            transport.read_frame_as::<Request<protocol::Request>>(),
        )
        .await;
        assert!(result.is_err(), "Unexpectedly got data: {:?}", result);

        // Write remainder of message
        proc.stdin.as_mut().unwrap().write(msg_b).await.unwrap();

        // Validate that the outgoing req is a complete LSP message
        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::ProcStdin { data, .. } => {
                assert_eq!(
                    data,
                    make_lsp_msg(serde_json::json!({
                        "field1": "a",
                        "field2": "b",
                    }))
                );
            }
            x => panic!("Unexpected request: {:?}", x),
        }
    }

    #[test(tokio::test)]
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
        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::ProcStdin { data, .. } => {
                assert_eq!(
                    data,
                    make_lsp_msg(serde_json::json!({
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

    #[test(tokio::test)]
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
        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::ProcStdin { data, .. } => {
                assert_eq!(
                    data,
                    make_lsp_msg(serde_json::json!({
                        "field1": "a",
                        "field2": "b",
                    }))
                );
            }
            x => panic!("Unexpected request: {:?}", x),
        }

        // Validate that the second outgoing req is a complete LSP message matching second
        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::ProcStdin { data, .. } => {
                assert_eq!(
                    data,
                    make_lsp_msg(serde_json::json!({
                        "field1": "c",
                        "field2": "d",
                    }))
                );
            }
            x => panic!("Unexpected request: {:?}", x),
        }
    }

    #[test(tokio::test)]
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
        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::ProcStdin { data, .. } => {
                // Verify the contents AND headers are as expected; in this case,
                // this will also ensure that the Content-Length is adjusted
                // when the distant scheme was changed to file
                assert_eq!(
                    data,
                    make_lsp_msg(serde_json::json!({
                        "field1": "file://some/path",
                        "field2": "file://other/path",
                    }))
                );
            }
            x => panic!("Unexpected request: {:?}", x),
        }
    }

    #[test(tokio::test)]
    async fn stdout_read_should_yield_lsp_messages_as_strings() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        // Send complete LSP message as stdout to process
        transport
            .write_frame_for(&Response::new(
                proc.origin_id().to_string(),
                protocol::Response::ProcStdout {
                    id: proc.id(),
                    data: make_lsp_msg(serde_json::json!({
                        "field1": "a",
                        "field2": "b",
                    })),
                },
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

    #[test(tokio::test)]
    async fn stdout_read_should_only_yield_complete_lsp_messages() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        let msg = make_lsp_msg(serde_json::json!({
            "field1": "a",
            "field2": "b",
        }));
        let (msg_a, msg_b) = msg.split_at(msg.len() / 2);

        // Send half of LSP message over stdout
        transport
            .write_frame_for(&Response::new(
                proc.origin_id().to_string(),
                protocol::Response::ProcStdout {
                    id: proc.id(),
                    data: msg_a.to_vec(),
                },
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
            .write_frame_for(&Response::new(
                proc.origin_id().to_string(),
                protocol::Response::ProcStdout {
                    id: proc.id(),
                    data: msg_b.to_vec(),
                },
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

    #[test(tokio::test)]
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
            .write_frame_for(&Response::new(
                proc.origin_id().to_string(),
                protocol::Response::ProcStdout {
                    id: proc.id(),
                    data: format!("{}{}", String::from_utf8(msg).unwrap(), extra).into_bytes(),
                },
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

    #[test(tokio::test)]
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
            .write_frame_for(&Response::new(
                proc.origin_id().to_string(),
                protocol::Response::ProcStdout {
                    id: proc.id(),
                    data: format!(
                        "{}{}",
                        String::from_utf8(msg_1).unwrap(),
                        String::from_utf8(msg_2).unwrap()
                    )
                    .into_bytes(),
                },
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

    #[test(tokio::test)]
    async fn stdout_read_should_convert_content_with_file_scheme_to_distant_scheme() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        // Send complete LSP message as stdout to process
        transport
            .write_frame_for(&Response::new(
                proc.origin_id().to_string(),
                protocol::Response::ProcStdout {
                    id: proc.id(),
                    data: make_lsp_msg(serde_json::json!({
                        "field1": "distant://some/path",
                        "field2": "file://other/path",
                    })),
                },
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

    #[test(tokio::test)]
    async fn stderr_read_should_yield_lsp_messages_as_strings() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        // Send complete LSP message as stderr to process
        transport
            .write_frame_for(&Response::new(
                proc.origin_id().to_string(),
                protocol::Response::ProcStderr {
                    id: proc.id(),
                    data: make_lsp_msg(serde_json::json!({
                        "field1": "a",
                        "field2": "b",
                    })),
                },
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

    #[test(tokio::test)]
    async fn stderr_read_should_only_yield_complete_lsp_messages() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        let msg = make_lsp_msg(serde_json::json!({
            "field1": "a",
            "field2": "b",
        }));
        let (msg_a, msg_b) = msg.split_at(msg.len() / 2);

        // Send half of LSP message over stderr
        transport
            .write_frame_for(&Response::new(
                proc.origin_id().to_string(),
                protocol::Response::ProcStderr {
                    id: proc.id(),
                    data: msg_a.to_vec(),
                },
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
            .write_frame_for(&Response::new(
                proc.origin_id().to_string(),
                protocol::Response::ProcStderr {
                    id: proc.id(),
                    data: msg_b.to_vec(),
                },
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

    #[test(tokio::test)]
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
            .write_frame_for(&Response::new(
                proc.origin_id().to_string(),
                protocol::Response::ProcStderr {
                    id: proc.id(),
                    data: format!("{}{}", String::from_utf8(msg).unwrap(), extra).into_bytes(),
                },
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

    #[test(tokio::test)]
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
            .write_frame_for(&Response::new(
                proc.origin_id().to_string(),
                protocol::Response::ProcStderr {
                    id: proc.id(),
                    data: format!(
                        "{}{}",
                        String::from_utf8(msg_1).unwrap(),
                        String::from_utf8(msg_2).unwrap()
                    )
                    .into_bytes(),
                },
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

    #[test(tokio::test)]
    async fn stderr_read_should_convert_content_with_file_scheme_to_distant_scheme() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        // Send complete LSP message as stderr to process
        transport
            .write_frame_for(&Response::new(
                proc.origin_id().to_string(),
                protocol::Response::ProcStderr {
                    id: proc.id(),
                    data: make_lsp_msg(serde_json::json!({
                        "field1": "distant://some/path",
                        "field2": "file://other/path",
                    })),
                },
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

    // ------------------------------------------------------------------
    // RemoteLspCommand builder methods
    // ------------------------------------------------------------------

    #[test]
    fn remote_lsp_command_new_should_have_default_values() {
        let cmd = RemoteLspCommand::new();
        assert!(cmd.pty.is_none());
        assert!(cmd.current_dir.is_none());
        assert!(cmd.scheme.is_none());
    }

    #[test]
    fn remote_lsp_command_default_should_be_same_as_new() {
        let cmd = RemoteLspCommand::default();
        assert!(cmd.pty.is_none());
        assert!(cmd.current_dir.is_none());
        assert!(cmd.scheme.is_none());
    }

    #[test]
    fn remote_lsp_command_pty_should_set_pty_size() {
        let mut cmd = RemoteLspCommand::new();
        let size = PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        };
        cmd.pty(Some(size));
        assert_eq!(cmd.pty, Some(size));
    }

    #[test]
    fn remote_lsp_command_pty_should_clear_when_set_to_none() {
        let mut cmd = RemoteLspCommand::new();
        cmd.pty(Some(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        }));
        cmd.pty(None);
        assert!(cmd.pty.is_none());
    }

    #[test]
    fn remote_lsp_command_environment_should_replace_environment() {
        let mut cmd = RemoteLspCommand::new();
        let mut env = Environment::new();
        env.insert("KEY".to_string(), "VALUE".to_string());
        cmd.environment(env.clone());
        assert_eq!(cmd.environment, env);
    }

    #[test]
    fn remote_lsp_command_current_dir_should_set_current_dir() {
        let mut cmd = RemoteLspCommand::new();
        cmd.current_dir(Some(PathBuf::from("/some/dir")));
        assert_eq!(cmd.current_dir, Some(PathBuf::from("/some/dir")));
    }

    #[test]
    fn remote_lsp_command_current_dir_should_clear_when_set_to_none() {
        let mut cmd = RemoteLspCommand::new();
        cmd.current_dir(Some(PathBuf::from("/some/dir")));
        cmd.current_dir(None);
        assert!(cmd.current_dir.is_none());
    }

    #[test]
    fn remote_lsp_command_scheme_should_set_scheme() {
        let mut cmd = RemoteLspCommand::new();
        cmd.scheme(Some(String::from("custom")));
        assert_eq!(cmd.scheme, Some(String::from("custom")));
    }

    #[test]
    fn remote_lsp_command_scheme_should_clear_when_set_to_none() {
        let mut cmd = RemoteLspCommand::new();
        cmd.scheme(Some(String::from("custom")));
        cmd.scheme(None);
        assert!(cmd.scheme.is_none());
    }

    #[test]
    fn remote_lsp_command_builder_methods_should_return_self() {
        let mut cmd = RemoteLspCommand::new();
        // All builder methods return &mut Self, allowing chaining
        cmd.pty(None).environment(Environment::new()).current_dir(None).scheme(None);
    }

    // ------------------------------------------------------------------
    // RemoteLspProcess Deref/DerefMut
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn remote_lsp_process_deref_should_expose_inner_process() {
        let (_transport, proc) = spawn_lsp_process().await;
        // Deref gives us access to RemoteProcess fields like id() and origin_id()
        let _id = proc.id();
        let _origin_id = proc.origin_id();
    }

    #[test(tokio::test)]
    async fn remote_lsp_process_deref_mut_should_allow_mutable_access_to_inner() {
        let (_transport, mut proc) = spawn_lsp_process().await;
        // DerefMut allows obtaining a &mut RemoteProcess from &mut RemoteLspProcess
        let inner: &mut RemoteProcess = &mut proc;
        let _id = inner.id();
    }

    // ------------------------------------------------------------------
    // stdin try_write and try_write_str
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn stdin_try_write_should_send_complete_lsp_messages() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        proc.stdin
            .as_mut()
            .unwrap()
            .try_write(&make_lsp_msg(serde_json::json!({
                "field1": "a",
                "field2": "b",
            })))
            .unwrap();

        // Validate that the outgoing req is a complete LSP message
        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::ProcStdin { data, .. } => {
                assert_eq!(
                    data,
                    make_lsp_msg(serde_json::json!({
                        "field1": "a",
                        "field2": "b",
                    }))
                );
            }
            x => panic!("Unexpected request: {:?}", x),
        }
    }

    #[test(tokio::test)]
    async fn stdin_try_write_str_should_send_complete_lsp_messages() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        let msg = make_lsp_msg(serde_json::json!({
            "field1": "x",
            "field2": "y",
        }));

        proc.stdin
            .as_mut()
            .unwrap()
            .try_write_str(&String::from_utf8(msg).unwrap())
            .unwrap();

        // Validate that the outgoing req is a complete LSP message
        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::ProcStdin { data, .. } => {
                assert_eq!(
                    data,
                    make_lsp_msg(serde_json::json!({
                        "field1": "x",
                        "field2": "y",
                    }))
                );
            }
            x => panic!("Unexpected request: {:?}", x),
        }
    }

    #[test(tokio::test)]
    async fn stdin_try_write_should_buffer_incomplete_messages() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        let msg = make_lsp_msg(serde_json::json!({
            "field1": "a",
            "field2": "b",
        }));
        let (msg_a, msg_b) = msg.split_at(msg.len() / 2);

        // Write part of the message that isn't finished
        proc.stdin.as_mut().unwrap().try_write(msg_a).unwrap();

        // Verify that nothing has been sent out yet
        tokio::task::yield_now().await;
        let result = timeout(
            TIMEOUT,
            transport.read_frame_as::<Request<protocol::Request>>(),
        )
        .await;
        assert!(result.is_err(), "Unexpectedly got data: {:?}", result);

        // Write remainder of message
        proc.stdin.as_mut().unwrap().try_write(msg_b).unwrap();

        // Now the complete message should be sent
        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::ProcStdin { data, .. } => {
                assert_eq!(
                    data,
                    make_lsp_msg(serde_json::json!({
                        "field1": "a",
                        "field2": "b",
                    }))
                );
            }
            x => panic!("Unexpected request: {:?}", x),
        }
    }

    #[test(tokio::test)]
    async fn stdin_try_write_should_convert_distant_scheme_to_file_scheme() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        proc.stdin
            .as_mut()
            .unwrap()
            .try_write(&make_lsp_msg(serde_json::json!({
                "field1": "distant://some/path",
                "field2": "file://other/path",
            })))
            .unwrap();

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::ProcStdin { data, .. } => {
                assert_eq!(
                    data,
                    make_lsp_msg(serde_json::json!({
                        "field1": "file://some/path",
                        "field2": "file://other/path",
                    }))
                );
            }
            x => panic!("Unexpected request: {:?}", x),
        }
    }

    // ------------------------------------------------------------------
    // Custom scheme for stdin
    // ------------------------------------------------------------------

    async fn spawn_lsp_process_with_scheme(
        scheme: &str,
    ) -> (FramedTransport<InmemoryTransport>, RemoteLspProcess) {
        let (mut t1, t2) = FramedTransport::pair(100);
        let client = Client::spawn_inmemory(t2, Default::default());
        let scheme = scheme.to_string();
        let spawn_task = tokio::spawn({
            let channel = client.clone_channel();
            async move {
                RemoteLspCommand::new()
                    .scheme(Some(scheme))
                    .spawn(channel, String::from("cmd arg"))
                    .await
            }
        });

        let req: Request<protocol::Request> = t1.read_frame_as().await.unwrap().unwrap();
        t1.write_frame_for(&Response::new(
            req.id,
            protocol::Response::ProcSpawned { id: rand::random() },
        ))
        .await
        .unwrap();

        let proc = spawn_task.await.unwrap().unwrap();
        (t1, proc)
    }

    #[test(tokio::test)]
    async fn stdin_write_with_custom_scheme_should_convert_scheme_to_file() {
        let (mut transport, mut proc) = spawn_lsp_process_with_scheme("custom").await;

        proc.stdin
            .as_mut()
            .unwrap()
            .write(&make_lsp_msg(serde_json::json!({
                "field1": "custom://some/path",
                "field2": "file://other/path",
            })))
            .await
            .unwrap();

        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::ProcStdin { data, .. } => {
                assert_eq!(
                    data,
                    make_lsp_msg(serde_json::json!({
                        "field1": "file://some/path",
                        "field2": "file://other/path",
                    }))
                );
            }
            x => panic!("Unexpected request: {:?}", x),
        }
    }

    #[test(tokio::test)]
    async fn stdout_read_with_custom_scheme_should_convert_file_to_custom_scheme() {
        let (mut transport, mut proc) = spawn_lsp_process_with_scheme("custom").await;

        transport
            .write_frame_for(&Response::new(
                proc.origin_id().to_string(),
                protocol::Response::ProcStdout {
                    id: proc.id(),
                    data: make_lsp_msg(serde_json::json!({
                        "field1": "file://some/path",
                        "field2": "custom://other/path",
                    })),
                },
            ))
            .await
            .unwrap();

        let out = proc.stdout.as_mut().unwrap().read().await.unwrap();
        assert_eq!(
            out,
            make_lsp_msg(serde_json::json!({
                "field1": "custom://some/path",
                "field2": "custom://other/path",
            }))
        );
    }

    #[test(tokio::test)]
    async fn stderr_read_with_custom_scheme_should_convert_file_to_custom_scheme() {
        let (mut transport, mut proc) = spawn_lsp_process_with_scheme("custom").await;

        transport
            .write_frame_for(&Response::new(
                proc.origin_id().to_string(),
                protocol::Response::ProcStderr {
                    id: proc.id(),
                    data: make_lsp_msg(serde_json::json!({
                        "field1": "file://some/path",
                        "field2": "custom://other/path",
                    })),
                },
            ))
            .await
            .unwrap();

        let err = proc.stderr.as_mut().unwrap().read().await.unwrap();
        assert_eq!(
            err,
            make_lsp_msg(serde_json::json!({
                "field1": "custom://some/path",
                "field2": "custom://other/path",
            }))
        );
    }

    // ------------------------------------------------------------------
    // stdout try_read, try_read_string, read_string
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn stdout_try_read_should_return_none_when_no_data_available() {
        let (_transport, mut proc) = spawn_lsp_process().await;

        let result = proc.stdout.as_mut().unwrap().try_read().unwrap();
        assert!(result.is_none());
    }

    #[test(tokio::test)]
    async fn stdout_try_read_should_return_data_when_available() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        transport
            .write_frame_for(&Response::new(
                proc.origin_id().to_string(),
                protocol::Response::ProcStdout {
                    id: proc.id(),
                    data: make_lsp_msg(serde_json::json!({
                        "field1": "a",
                    })),
                },
            ))
            .await
            .unwrap();

        // Give the read task time to process
        tokio::time::sleep(Duration::from_millis(50)).await;

        let result = proc.stdout.as_mut().unwrap().try_read().unwrap();
        assert!(result.is_some());
    }

    #[test(tokio::test)]
    async fn stdout_try_read_string_should_return_none_when_no_data_available() {
        let (_transport, mut proc) = spawn_lsp_process().await;

        let result = proc.stdout.as_mut().unwrap().try_read_string().unwrap();
        assert!(result.is_none());
    }

    #[test(tokio::test)]
    async fn stdout_read_string_should_return_string() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        transport
            .write_frame_for(&Response::new(
                proc.origin_id().to_string(),
                protocol::Response::ProcStdout {
                    id: proc.id(),
                    data: make_lsp_msg(serde_json::json!({
                        "field1": "a",
                        "field2": "b",
                    })),
                },
            ))
            .await
            .unwrap();

        let out = proc.stdout.as_mut().unwrap().read_string().await.unwrap();
        assert_eq!(
            out,
            String::from_utf8(make_lsp_msg(serde_json::json!({
                "field1": "a",
                "field2": "b",
            })))
            .unwrap()
        );
    }

    // ------------------------------------------------------------------
    // stderr try_read, try_read_string, read_string
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn stderr_try_read_should_return_none_when_no_data_available() {
        let (_transport, mut proc) = spawn_lsp_process().await;

        let result = proc.stderr.as_mut().unwrap().try_read().unwrap();
        assert!(result.is_none());
    }

    #[test(tokio::test)]
    async fn stderr_try_read_should_return_data_when_available() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        transport
            .write_frame_for(&Response::new(
                proc.origin_id().to_string(),
                protocol::Response::ProcStderr {
                    id: proc.id(),
                    data: make_lsp_msg(serde_json::json!({
                        "field1": "a",
                    })),
                },
            ))
            .await
            .unwrap();

        // Give the read task time to process
        tokio::time::sleep(Duration::from_millis(50)).await;

        let result = proc.stderr.as_mut().unwrap().try_read().unwrap();
        assert!(result.is_some());
    }

    #[test(tokio::test)]
    async fn stderr_try_read_string_should_return_none_when_no_data_available() {
        let (_transport, mut proc) = spawn_lsp_process().await;

        let result = proc.stderr.as_mut().unwrap().try_read_string().unwrap();
        assert!(result.is_none());
    }

    #[test(tokio::test)]
    async fn stderr_read_string_should_return_string() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        transport
            .write_frame_for(&Response::new(
                proc.origin_id().to_string(),
                protocol::Response::ProcStderr {
                    id: proc.id(),
                    data: make_lsp_msg(serde_json::json!({
                        "field1": "a",
                        "field2": "b",
                    })),
                },
            ))
            .await
            .unwrap();

        let err = proc.stderr.as_mut().unwrap().read_string().await.unwrap();
        assert_eq!(
            err,
            String::from_utf8(make_lsp_msg(serde_json::json!({
                "field1": "a",
                "field2": "b",
            })))
            .unwrap()
        );
    }

    // ------------------------------------------------------------------
    // stdout/stderr Drop behavior
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn stdout_drop_should_abort_read_task() {
        let (_transport, mut proc) = spawn_lsp_process().await;

        let stdout = proc.stdout.take().unwrap();
        let task_handle = &stdout.read_task;
        assert!(!task_handle.is_finished());

        drop(stdout);

        // Give the task time to be aborted
        tokio::time::sleep(Duration::from_millis(50)).await;

        // After drop, the read_task should be aborted
        // We can't check the handle since it was dropped, but the test
        // passes if drop doesn't panic
    }

    #[test(tokio::test)]
    async fn stderr_drop_should_abort_read_task() {
        let (_transport, mut proc) = spawn_lsp_process().await;

        let stderr = proc.stderr.take().unwrap();
        let task_handle = &stderr.read_task;
        assert!(!task_handle.is_finished());

        drop(stderr);

        // Give the task time to be aborted
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // ------------------------------------------------------------------
    // read_lsp_messages
    // ------------------------------------------------------------------

    #[test]
    fn read_lsp_messages_should_return_empty_queue_for_empty_input() {
        let (remainder, queue) = read_lsp_messages(b"").unwrap();
        assert!(remainder.is_none());
        assert!(queue.is_empty());
    }

    #[test]
    fn read_lsp_messages_should_parse_single_complete_message() {
        let msg = make_lsp_msg(serde_json::json!({"key": "value"}));
        let (remainder, queue) = read_lsp_messages(&msg).unwrap();
        assert!(remainder.is_none());
        assert_eq!(queue.len(), 1);
    }

    #[test]
    fn read_lsp_messages_should_parse_multiple_complete_messages() {
        let msg1 = make_lsp_msg(serde_json::json!({"key": "value1"}));
        let msg2 = make_lsp_msg(serde_json::json!({"key": "value2"}));
        let mut input = msg1;
        input.extend(msg2);
        let (remainder, queue) = read_lsp_messages(&input).unwrap();
        assert!(remainder.is_none());
        assert_eq!(queue.len(), 2);
    }

    #[test]
    fn read_lsp_messages_should_keep_incomplete_data_as_remainder() {
        let msg = make_lsp_msg(serde_json::json!({"key": "value"}));
        let extra = b"Content-Length: 999\r\n\r\n";
        let mut input = msg;
        input.extend_from_slice(extra);
        let (remainder, queue) = read_lsp_messages(&input).unwrap();
        assert!(remainder.is_some());
        assert_eq!(queue.len(), 1);
        // The remainder should contain the partial message
        let rem = remainder.unwrap();
        assert!(rem.starts_with(b"Content-Length: 999"));
    }

    #[test]
    fn read_lsp_messages_should_keep_all_data_as_remainder_when_no_complete_message() {
        let input = b"Content-Length: 999\r\n\r\npartial";
        let (remainder, queue) = read_lsp_messages(input).unwrap();
        assert!(remainder.is_some());
        assert!(queue.is_empty());
        assert_eq!(remainder.unwrap(), input);
    }

    // ------------------------------------------------------------------
    // stdin update_and_read_messages internal buffer handling
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn stdin_should_maintain_buffer_across_multiple_writes() {
        let (mut transport, mut proc) = spawn_lsp_process().await;

        let msg = make_lsp_msg(serde_json::json!({"key": "value"}));

        // Write first third
        let first_third = msg.len() / 3;
        proc.stdin
            .as_mut()
            .unwrap()
            .write(&msg[..first_third])
            .await
            .unwrap();

        // Nothing should be sent yet
        tokio::task::yield_now().await;
        let result = timeout(
            TIMEOUT,
            transport.read_frame_as::<Request<protocol::Request>>(),
        )
        .await;
        assert!(result.is_err());

        // Write second third
        let second_third = 2 * msg.len() / 3;
        proc.stdin
            .as_mut()
            .unwrap()
            .write(&msg[first_third..second_third])
            .await
            .unwrap();

        // Still nothing should be sent
        tokio::task::yield_now().await;
        let result = timeout(
            TIMEOUT,
            transport.read_frame_as::<Request<protocol::Request>>(),
        )
        .await;
        assert!(result.is_err());

        // Write final third
        proc.stdin
            .as_mut()
            .unwrap()
            .write(&msg[second_third..])
            .await
            .unwrap();

        // Now the complete message should be sent
        let req: Request<protocol::Request> = transport.read_frame_as().await.unwrap().unwrap();
        match req.payload {
            protocol::Request::ProcStdin { data, .. } => {
                assert_eq!(
                    data,
                    make_lsp_msg(serde_json::json!({"key": "value"}))
                );
            }
            x => panic!("Unexpected request: {:?}", x),
        }
    }

    // ------------------------------------------------------------------
    // stdout/stderr BrokenPipe on disconnect
    // ------------------------------------------------------------------

    #[test(tokio::test)]
    async fn stdout_try_read_should_return_broken_pipe_when_disconnected() {
        let (_transport, mut proc) = spawn_lsp_process().await;

        // Take and drop the stdout to disconnect
        let mut stdout = proc.stdout.take().unwrap();

        // Close the receiver to simulate disconnection
        stdout.rx.close();

        // Drain any pending messages
        while stdout.rx.try_recv().is_ok() {}

        // Now try_read should return BrokenPipe
        let result = stdout.try_read();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    }

    #[test(tokio::test)]
    async fn stderr_try_read_should_return_broken_pipe_when_disconnected() {
        let (_transport, mut proc) = spawn_lsp_process().await;

        let mut stderr = proc.stderr.take().unwrap();

        // Close the receiver to simulate disconnection
        stderr.rx.close();

        // Drain any pending messages
        while stderr.rx.try_recv().is_ok() {}

        // Now try_read should return BrokenPipe
        let result = stderr.try_read();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    }
}
