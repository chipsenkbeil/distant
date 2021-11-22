use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    runtime::Handle,
};

/// Sends JSON messages over stdout
pub struct MsgSender<'a>(InnerMsgSender<'a>);

impl<'a> MsgSender<'a> {
    /// Creates a new sender from the asynchronous writer
    pub fn from_async<T>(writer: T) -> Self
    where
        T: AsyncWrite + Unpin + 'a,
    {
        Self(InnerMsgSender::AsyncSender(Box::new(writer)))
    }

    /// Creates a new sender from the synchronous writer
    pub fn from_sync<T>(writer: T) -> Self
    where
        T: Write + 'a,
    {
        Self(InnerMsgSender::SyncSender(Box::new(writer)))
    }
}

enum InnerMsgSender<'a> {
    AsyncSender(Box<dyn AsyncWrite + Unpin + 'a>),
    SyncSender(Box<dyn Write + 'a>),
}

impl<'a> InnerMsgSender<'a> {
    pub fn send_blocking<T>(&mut self, ser: &T) -> io::Result<()>
    where
        T: Serialize,
    {
        let msg = format!("{}\n", serde_json::to_string(ser)?);

        match self {
            InnerMsgSender::AsyncSender(writer) => {
                Handle::current().block_on(writer.write_all(msg.as_bytes()))?
            }
            InnerMsgSender::SyncSender(writer) => writer.write_all(msg.as_bytes())?,
        }

        Ok(())
    }

    pub async fn send<T>(&mut self, ser: &T) -> io::Result<()>
    where
        T: Serialize,
    {
        let msg = format!("{}\n", serde_json::to_string(ser)?);

        match self {
            InnerMsgSender::AsyncSender(writer) => writer.write_all(msg.as_bytes()).await?,
            InnerMsgSender::SyncSender(writer) => {
                Handle::current()
                    .spawn_blocking(|| writer.write_all(msg.as_bytes()))
                    .await??
            }
        }

        Ok(())
    }
}

/// Receives JSON messages over stdin
pub struct MsgReceiver<'a>(InnerMsgReceiver<'a>);

enum InnerMsgReceiver<'a> {
    AsyncReceiver(Box<dyn AsyncRead + Unpin + 'a>),
    SyncReceiver(Box<dyn Read + 'a>),
}
