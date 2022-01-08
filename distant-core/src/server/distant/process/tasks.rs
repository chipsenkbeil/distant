use super::{
    ExitStatus, InputChannel, OutputChannel, Process, ProcessKiller, ProcessStderr, ProcessStdin,
    ProcessStdout, Wait,
};
use crate::constants::{MAX_PIPE_CHUNK_SIZE, READ_PAUSE_MILLIS};
use std::{future::Future, pin::Pin};
use tokio::{
    io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::mpsc,
    task::JoinHandle,
};

pub fn spawn_wait_task<W>(writer: W, buf: usize) -> JoinHandle<io::Result<()>>
where
    W: AsyncWrite + Send + Unpin + 'static,
{
    let (tx, rx) = mpsc::channel(buf);
    let task = tokio::spawn(wait_handler(writer, rx));
    (task, Box::new(tx))
}

async fn wait_handler(
    mut wait: Wait,
    stdin_task: JoinHandle<()>,
    stdout_task: JoinHandle<()>,
    stderr_task: JoinHandle<()>,
) -> io::Result<ExitStatus> {
    let mut status = wait.resolve().await?;

    stdin_task.abort();
    let _ = stdout_task.await;
    let _ = stderr_task.await;

    if status.success && status.code.is_none() {
        status.code = Some(0);
    }
    Ok(status)
}

pub fn spawn_read_task<R>(
    reader: R,
    buf: usize,
) -> (JoinHandle<io::Result<()>>, Box<dyn OutputChannel>)
where
    R: AsyncRead + Send + Unpin + 'static,
{
    let (tx, rx) = mpsc::channel(buf);
    let task = tokio::spawn(read_handler(reader, tx));
    (task, Box::new(rx))
}

/// Continually reads from some reader and fowards to the provided sender until the reader
/// or channel is closed
async fn read_handler<R>(mut reader: R, channel: mpsc::Sender<Vec<u8>>) -> io::Result<()>
where
    R: AsyncRead + Unpin,
{
    let mut buf: [u8; MAX_PIPE_CHUNK_SIZE] = [0; MAX_PIPE_CHUNK_SIZE];
    loop {
        match reader.read(&mut buf).await {
            Ok(n) if n > 0 => {
                let _ = channel.send(buf[..n].to_vec()).await.map_err(|_| {
                    io::Error::new(io::ErrorKind::BrokenPipe, "Output channel closed")
                })?;

                // Pause to allow buffer to fill up a little bit, avoiding
                // spamming with a lot of smaller responses
                tokio::time::sleep(tokio::time::Duration::from_millis(READ_PAUSE_MILLIS)).await;
            }
            Ok(_) => return Ok(()),
            Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                // Pause to allow buffer to fill up a little bit, avoiding
                // spamming with a lot of smaller responses
                tokio::time::sleep(tokio::time::Duration::from_millis(READ_PAUSE_MILLIS)).await;
            }
            Err(x) => return Err(x),
        }
    }
}

pub fn spawn_write_task<W>(
    writer: W,
    buf: usize,
) -> (JoinHandle<io::Result<()>>, Box<dyn InputChannel>)
where
    W: AsyncWrite + Send + Unpin + 'static,
{
    let (tx, rx) = mpsc::channel(buf);
    let task = tokio::spawn(write_handler(writer, rx));
    (task, Box::new(tx))
}

/// Continually writes to some writer by reading data from a provided receiver until the receiver
/// or writer is closed
async fn write_handler<W>(mut writer: W, mut channel: mpsc::Receiver<Vec<u8>>) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    while let Some(data) = channel.recv().await {
        let _ = writer.write_all(&data).await?;
    }
    Ok(())
}
