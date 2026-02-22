use std::ffi::OsStr;
use std::io;
use std::time::Duration;

use derive_more::{From, TryInto};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient, NamedPipeServer};

// Equivalent to winapi::shared::winerror::ERROR_PIPE_BUSY
// DWORD -> c_uLong -> u32
const ERROR_PIPE_BUSY: u32 = 231;

// Time between attempts to connect to a busy pipe
const BUSY_PIPE_SLEEP_DURATION: Duration = Duration::from_millis(50);

/// Represents a named pipe from either a client or server perspective
#[derive(From, TryInto)]
pub enum NamedPipe {
    Client(NamedPipeClient),
    Server(NamedPipeServer),
}

impl NamedPipe {
    /// Returns true if the underlying named pipe is a client named pipe
    pub fn is_client(&self) -> bool {
        matches!(self, Self::Client(_))
    }

    /// Returns a reference to the underlying named client pipe
    pub fn as_client(&self) -> Option<&NamedPipeClient> {
        match self {
            Self::Client(x) => Some(x),
            _ => None,
        }
    }

    /// Returns a mutable reference to the underlying named client pipe
    pub fn as_mut_client(&mut self) -> Option<&mut NamedPipeClient> {
        match self {
            Self::Client(x) => Some(x),
            _ => None,
        }
    }

    /// Consumes and returns the underlying named client pipe
    pub fn into_client(self) -> Option<NamedPipeClient> {
        match self {
            Self::Client(x) => Some(x),
            _ => None,
        }
    }

    /// Returns true if the underlying named pipe is a server named pipe
    pub fn is_server(&self) -> bool {
        matches!(self, Self::Server(_))
    }

    /// Returns a reference to the underlying named server pipe
    pub fn as_server(&self) -> Option<&NamedPipeServer> {
        match self {
            Self::Server(x) => Some(x),
            _ => None,
        }
    }

    /// Returns a mutable reference to the underlying named server pipe
    pub fn as_mut_server(&mut self) -> Option<&mut NamedPipeServer> {
        match self {
            Self::Server(x) => Some(x),
            _ => None,
        }
    }

    /// Consumes and returns the underlying named server pipe
    pub fn into_server(self) -> Option<NamedPipeServer> {
        match self {
            Self::Server(x) => Some(x),
            _ => None,
        }
    }

    /// Attempts to connect as a client pipe
    pub(super) async fn connect_as_client(addr: &OsStr) -> io::Result<Self> {
        let pipe = loop {
            match ClientOptions::new().open(addr) {
                Ok(client) => break client,
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => (),
                Err(e) => return Err(e),
            }

            tokio::time::sleep(BUSY_PIPE_SLEEP_DURATION).await;
        };

        Ok(NamedPipe::from(pipe))
    }
}
