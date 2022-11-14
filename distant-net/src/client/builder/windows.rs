use super::Connector;
use crate::common::WindowsPipeTransport;
use async_trait::async_trait;
use std::ffi::OsString;
use std::io;

/// Implementation of [`Connector`] to support connecting via a Windows named pipe.
pub struct WindowsPipeConnector {
    addr: OsString,
    pub(crate) local: bool,
}

impl WindowsPipeConnector {
    /// Creates a new connector for a non-local pipe using the given `addr`.
    pub fn new(addr: impl Into<OsString>) -> Self {
        Self { addr: addr.into(), local: false }
    }

    /// Creates a new connector for a local pipe using the given `name`.
    pub fn local(name: impl Into<OsString>) -> Self {
        Self { addr: name.into(), local: true }
    }
}

impl<T: Into<OsString>> From<T> for WindowsPipeConnector {
    fn from(addr: T) -> Self {
        Self::new(addr)
    }
}

#[async_trait]
impl Connector for WindowsPipeConnector {
    type Transport = WindowsPipeTransport;

    async fn connect(self) -> io::Result<Self::Transport> {
        if self.local {
                let mut full_addr = OsString::from(r"\\.\pipe\");
                full_addr.push(self.addr.as_ref());
                WindowsPipeTransport::connect(full_addr).await
            } else {
                WindowsPipeTransport::connect(self.addr).await
            }
    }
}
