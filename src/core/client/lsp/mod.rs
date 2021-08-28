use super::{RemoteProcess, RemoteProcessError, RemoteStderr, RemoteStdin, RemoteStdout};
use crate::core::{client::Session, net::DataStream};
use std::{
    fmt::Write,
    io::{self, Cursor, Read},
    ops::{Deref, DerefMut},
};

mod data;
pub use data::*;

/// Represents an LSP server process on a remote machine
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
        tenant: String,
        session: Session<T>,
        cmd: String,
        args: Vec<String>,
    ) -> Result<Self, RemoteProcessError>
    where
        T: DataStream + 'static,
    {
        let mut inner = RemoteProcess::spawn(tenant, session, cmd, args).await?;
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
pub struct RemoteLspStdin {
    inner: RemoteStdin,
    buf: Option<String>,
}

impl RemoteLspStdin {
    pub fn new(inner: RemoteStdin) -> Self {
        Self { inner, buf: None }
    }

    /// Writes data to the stdin of a specific remote process
    pub async fn write(&mut self, data: &str) -> io::Result<()> {
        let mut queue = Vec::new();

        // Create or insert into our buffer
        match &mut self.buf {
            Some(buf) => buf.push_str(data),
            None => self.buf = Some(data.to_string()),
        }

        // Read LSP messages from our internal buffer
        let buf = self.buf.take().unwrap();
        let mut cursor = Cursor::new(buf);
        while let Ok(data) = LspData::from_buf_reader(&mut cursor) {
            queue.push(data);
        }

        // Keep remainder of string not processed as LSP message in buffer
        if (cursor.position() as usize) < cursor.get_ref().len() {
            let mut buf = String::new();
            cursor.read_to_string(&mut buf)?;
            self.buf = Some(buf);
        }

        // Process and then send out each LSP message in our queue
        for mut data in queue {
            // Convert distant:// to file://
            data.mut_content().convert_distant_scheme_to_local();
            self.inner.write(&data.to_string()).await?;
        }

        Ok(())
    }
}

/// A handle to a remote LSP process' standard output (stdout)
pub struct RemoteLspStdout {
    inner: RemoteStdout,
    buf: Option<String>,
}

impl RemoteLspStdout {
    pub fn new(inner: RemoteStdout) -> Self {
        Self { inner, buf: None }
    }

    pub async fn read(&mut self) -> io::Result<String> {
        let mut queue = Vec::new();
        let data = self.inner.read().await?;

        // Create or insert into our buffer
        match &mut self.buf {
            Some(buf) => buf.push_str(&data),
            None => self.buf = Some(data),
        }

        // Read LSP messages from our internal buffer
        let buf = self.buf.take().unwrap();
        let mut cursor = Cursor::new(buf);
        while let Ok(data) = LspData::from_buf_reader(&mut cursor) {
            queue.push(data);
        }

        // Keep remainder of string not processed as LSP message in buffer
        if (cursor.position() as usize) < cursor.get_ref().len() {
            let mut buf = String::new();
            cursor.read_to_string(&mut buf)?;
            self.buf = Some(buf);
        }

        // Process and then add each LSP message as output
        let mut out = String::new();
        for mut data in queue {
            // Convert file:// to distant://
            data.mut_content().convert_local_scheme_to_distant();
            write!(&mut out, "{}", data).unwrap();
        }

        Ok(out)
    }
}

/// A handle to a remote LSP process' stderr
pub struct RemoteLspStderr {
    inner: RemoteStderr,
    buf: Option<String>,
}

impl RemoteLspStderr {
    pub fn new(inner: RemoteStderr) -> Self {
        Self { inner, buf: None }
    }

    pub async fn read(&mut self) -> io::Result<String> {
        let mut queue = Vec::new();
        let data = self.inner.read().await?;

        // Create or insert into our buffer
        match &mut self.buf {
            Some(buf) => buf.push_str(&data),
            None => self.buf = Some(data),
        }

        // Read LSP messages from our internal buffer
        let buf = self.buf.take().unwrap();
        let mut cursor = Cursor::new(buf);
        while let Ok(data) = LspData::from_buf_reader(&mut cursor) {
            queue.push(data);
        }

        // Keep remainder of string not processed as LSP message in buffer
        if (cursor.position() as usize) < cursor.get_ref().len() {
            let mut buf = String::new();
            cursor.read_to_string(&mut buf)?;
            self.buf = Some(buf);
        }

        // Process and then add each LSP message as output
        let mut out = String::new();
        for mut data in queue {
            // Convert file:// to distant://
            data.mut_content().convert_local_scheme_to_distant();
            write!(&mut out, "{}", data).unwrap();
        }

        Ok(out)
    }
}
