use derive_more::{From, TryInto};
use std::{
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    io::{self, AsyncRead, AsyncWrite, ReadBuf},
    net::windows::named_pipe::{NamedPipeClient, NamedPipeServer},
};

#[derive(From, TryInto)]
pub enum NamedPipe {
    Client(NamedPipeClient),
    Server(NamedPipeServer),
}

impl NamedPipe {
    pub fn as_client(&self) -> Option<&NamedPipeClient> {
        match self {
            Self::Client(x) => Some(x),
            _ => None,
        }
    }

    pub fn as_mut_client(&mut self) -> Option<&mut NamedPipeClient> {
        match self {
            Self::Client(x) => Some(x),
            _ => None,
        }
    }

    pub fn into_client(self) -> Option<NamedPipeClient> {
        match self {
            Self::Client(x) => Some(x),
            _ => None,
        }
    }

    pub fn as_server(&self) -> Option<&NamedPipeServer> {
        match self {
            Self::Server(x) => Some(x),
            _ => None,
        }
    }

    pub fn as_mut_server(&mut self) -> Option<&mut NamedPipeServer> {
        match self {
            Self::Server(x) => Some(x),
            _ => None,
        }
    }

    pub fn into_server(self) -> Option<NamedPipeServer> {
        match self {
            Self::Server(x) => Some(x),
            _ => None,
        }
    }
}
impl AsyncRead for NamedPipe {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match Pin::get_mut(self) {
            Self::Client(x) => Pin::new(x).poll_read(cx, buf),
            Self::Server(x) => Pin::new(x).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for NamedPipe {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        match Pin::get_mut(self) {
            Self::Client(x) => Pin::new(x).poll_write(cx, buf),
            Self::Server(x) => Pin::new(x).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        match Pin::get_mut(self) {
            Self::Client(x) => Pin::new(x).poll_flush(cx),
            Self::Server(x) => Pin::new(x).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        match Pin::get_mut(self) {
            Self::Client(x) => Pin::new(x).poll_shutdown(cx),
            Self::Server(x) => Pin::new(x).poll_shutdown(cx),
        }
    }
}
