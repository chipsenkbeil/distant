use crate::DataStream;
use std::{
    ffi::{OsStr, OsString},
    fmt, io,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer},
};

// Equivalent to winapi::shared::winerror::ERROR_PIPE_BUSY
// DWORD -> c_uLong -> u32
const ERROR_PIPE_BUSY: u32 = 231;

// Time between attempts to connect to a busy pipe
const BUSY_PIPE_SLEEP_MILLIS: u64 = 50;

mod pipe;
pub use pipe::NamedPipe;

/// Represents a data stream for a Windows pipe (client or server)
pub struct WindowsPipeStream {
    pub(crate) addr: OsString,
    pub(crate) inner: NamedPipe,
}

impl WindowsPipeStream {
    /// Establishes a connection to the pipe with the specified name, using the
    /// name for a local pipe address in the form of `\\.\pipe\my_pipe_name` where
    /// `my_pipe_name` is provided to this function
    pub async fn connect_local(name: impl AsRef<OsStr>) -> io::Result<Self> {
        let mut addr = OsString::from(r"\\.\pipe\");
        addr.push(name.as_ref());
        Self::connect(addr).await
    }

    /// Establishes a connection to the pipe at the specified address
    ///
    /// Address may be something like `\.\pipe\my_pipe_name`
    pub async fn connect(addr: impl Into<OsString>) -> io::Result<Self> {
        let addr = addr.into();

        let pipe = loop {
            match ClientOptions::new().open(&addr) {
                Ok(client) => break client,
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => (),
                Err(e) => return Err(e),
            }

            tokio::time::sleep(Duration::from_millis(BUSY_PIPE_SLEEP_MILLIS)).await;
        };

        Ok(Self { addr, inner: pipe })
    }

    /// Returns the addr that the listener is bound to
    pub fn addr(&self) -> &OsStr {
        &self.addr
    }
}

impl fmt::Debug for WindowsPipeStream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WindowsPipeStream")
            .field("addr", &self.addr)
            .finish()
    }
}

impl DataStream for WindowsPipeStream {
    type Read = tokio::io::ReadHalf<WindowsPipeStream>;
    type Write = tokio::io::WriteHalf<WindowsPipeStream>;

    fn into_split(self) -> (Self::Read, Self::Write) {
        tokio::io::split(self)
    }
}

impl AsyncRead for WindowsPipeStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for WindowsPipeStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
