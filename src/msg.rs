use serde::{de::DeserializeOwned, Serialize};
use std::{
    future::Future,
    io::{self, Write},
    pin::Pin,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt},
    runtime::Handle,
};

/// Sends JSON messages over stdout
pub struct MsgSender(InnerMsgSender);

impl MsgSender {
    /// Creates a new sender from the asynchronous writer
    pub fn from_async<F>(f: F) -> Self
    where
        F: Fn(&[u8]) -> Pin<Box<dyn Future<Output = io::Result<()>>>> + 'static,
    {
        Self(InnerMsgSender::AsyncSender(Box::new(f)))
    }

    pub fn from_async_stdout() -> Self {
        let mut writer = tokio::io::stdout();

        async fn do_write(writer: &mut tokio::io::Stdout, output: &[u8]) -> io::Result<()> {
            let _ = writer.write_all(output).await?;
            let _ = writer.flush().await?;
            Ok(())
        }

        Self::from_async(move |output| Box::pin(do_write(&mut writer, output)))
    }

    /// Creates a new sender from the synchronous writer
    pub fn from_sync<F>(f: F) -> Self
    where
        F: Fn(&[u8]) -> io::Result<()> + 'static,
    {
        Self(InnerMsgSender::SyncSender(Box::new(f)))
    }

    pub fn from_sync_stdout() -> Self {
        let mut writer = std::io::stdout();
        Self::from_sync(move |output| {
            let _ = writer.write_all(output)?;
            let _ = writer.flush()?;
            Ok(())
        })
    }

    pub fn send_blocking<T>(&mut self, ser: &T) -> io::Result<()>
    where
        T: Serialize,
    {
        self.0.send_blocking(ser)
    }
}

enum InnerMsgSender {
    AsyncSender(Box<dyn FnMut(&[u8]) -> Pin<Box<dyn Future<Output = io::Result<()>>>>>),
    SyncSender(Box<dyn FnMut(&[u8]) -> io::Result<()>>),
}

impl InnerMsgSender {
    pub fn send_blocking<T>(&mut self, ser: &T) -> io::Result<()>
    where
        T: Serialize,
    {
        let msg = format!("{}\n", serde_json::to_string(ser)?);

        match self {
            InnerMsgSender::AsyncSender(write) => Handle::current().block_on(write(msg.as_bytes())),
            InnerMsgSender::SyncSender(write) => write(msg.as_bytes()),
        }
    }
}

/// Receives JSON messages over stdin
pub struct MsgReceiver(InnerMsgReceiver);

impl MsgReceiver {
    pub fn from_async<F>(f: F) -> Self
    where
        F: FnMut(&mut String) -> Pin<Box<dyn Future<Output = io::Result<()>>>> + 'static,
    {
        Self(InnerMsgReceiver::AsyncReceiver(Box::new(move |input| {
            f(input)
        })))
    }

    pub fn from_async_stdin() -> Self {
        let mut reader = tokio::io::BufReader::new(tokio::io::stdin());

        async fn do_read<T>(
            reader: &mut tokio::io::BufReader<T>,
            buf: &mut String,
        ) -> io::Result<()>
        where
            T: AsyncRead + Unpin,
        {
            let _ = reader.read_line(buf).await?;
            Ok(())
        }

        Self::from_async(move |input| Box::pin(do_read(&mut reader, input)))
    }

    pub fn from_sync<F>(f: F) -> Self
    where
        F: FnMut(&mut String) -> io::Result<()> + 'static,
    {
        Self(InnerMsgReceiver::SyncReceiver(Box::new(f)))
    }

    pub fn from_sync_stdin() -> Self {
        let mut reader = std::io::stdin();
        Self::from_sync(move |input| {
            let _ = reader.read_line(input)?;
            Ok(())
        })
    }

    pub fn recv_blocking<T>(&mut self) -> io::Result<T>
    where
        T: DeserializeOwned,
    {
        self.0.recv_blocking()
    }
}

enum InnerMsgReceiver {
    AsyncReceiver(Box<dyn FnMut(&mut String) -> Pin<Box<dyn Future<Output = io::Result<()>>>>>),
    SyncReceiver(Box<dyn FnMut(&mut String) -> io::Result<()>>),
}

impl InnerMsgReceiver {
    pub fn recv_blocking<T>(&mut self) -> io::Result<T>
    where
        T: DeserializeOwned,
    {
        let mut input = String::new();

        // Continue reading input until we get a match, only if reported that current input
        // is a partial match
        let data: T = loop {
            // Read in another line of input
            match self {
                InnerMsgReceiver::AsyncReceiver(read) => {
                    Handle::current().block_on(read(&mut input))?
                }
                InnerMsgReceiver::SyncReceiver(read) => read(&mut input)?,
            };

            // Attempt to parse current input as type, yielding it on success, continuing to read
            // more input if error is unexpected EOF (meaning we are partially reading json), and
            // failing if we get any other error
            match serde_json::from_str(&input) {
                Ok(data) => break data,
                Err(x) if x.is_eof() => continue,
                Err(x) => Err(x)?,
            }
        };

        Ok(data)
    }
}
