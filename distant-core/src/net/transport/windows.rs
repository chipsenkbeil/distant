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

impl DataStream for NamedPipeClient {
    type Read = tokio::io::ReadHalf<NamedPipeClient>;
    type Write = tokio::io::WriteHalf<NamedPipeClient>;

    fn into_split(self) -> (Self::Read, Self::Write) {
        tokio::io::split(self)
    }
}

impl DataStream for NamedPipeServer {
    type Read = tokio::io::ReadHalf<NamedPipeServer>;
    type Write = tokio::io::WriteHalf<NamedPipeServer>;

    fn into_split(self) -> (Self::Read, Self::Write) {
        tokio::io::split(self)
    }
}

impl<U: Codec> Transport<NamedPipeClient, U> {
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
        Ok(Transport::new(client, codec))
    }
}
