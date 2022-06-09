use crate::net::{Codec, DataStream, Transport};
use std::{path::Path, time::Duration};
use tokio::{
    io,
    net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer},
};

// Equivalent to winapi::shared::winerror::ERROR_PIPE_BUSY
// DWORD -> c_uLong -> u32
const ERROR_PIPE_BUSY: u32 = 231;

// Time between attempts to connect to a busy pipe
const BUSY_PIPE_SLEEP_MILLIS: u64 = 50;

mod pipe;
pub use pipe::NamedPipe;

impl_async_newtype!(WindowsPipeStream -> NamedPipe);

impl From<NamedPipeClient> for WindowsPipeStream {
    fn from(client: NamedPipeClient) -> Self {
        Self(NamedPipe::Client(client))
    }
}

impl From<NamedPipeServer> for WindowsPipeStream {
    fn from(server: NamedPipeServer) -> Self {
        Self(NamedPipe::Server(server))
    }
}

impl DataStream for WindowsPipeStream {
    type Read = tokio::io::ReadHalf<WindowsPipeStream>;
    type Write = tokio::io::WriteHalf<WindowsPipeStream>;

    fn into_split(self) -> (Self::Read, Self::Write) {
        tokio::io::split(self)
    }
}

impl<U: Codec> Transport<WindowsPipeStream, U> {
    /// Establishes a connection to the pipe at the specified path and uses the provided codec
    /// for transportation
    ///
    /// Path may be something like `\.\pipe\my_pipe_name`
    pub async fn connect(path: impl AsRef<Path>, codec: U) -> io::Result<Self> {
        let client = loop {
            match ClientOptions::new().open(path.as_ref()) {
                Ok(client) => break client,
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => (),
                Err(e) => return Err(e),
            }

            tokio::time::sleep(Duration::from_millis(BUSY_PIPE_SLEEP_MILLIS)).await;
        };
        Ok(Transport::new(WindowsPipeStream::from(client), codec))
    }
}
