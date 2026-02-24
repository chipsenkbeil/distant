use std::io;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::constants::{MAX_PIPE_CHUNK_SIZE, READ_PAUSE_DURATION};

pub fn spawn_read_task<R>(
    reader: R,
    buf: usize,
) -> (JoinHandle<io::Result<()>>, mpsc::Receiver<Vec<u8>>)
where
    R: AsyncRead + Send + Unpin + 'static,
{
    let (tx, rx) = mpsc::channel(buf);
    let task = tokio::spawn(read_handler(reader, tx));
    (task, rx)
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
                channel.send(buf[..n].to_vec()).await.map_err(|_| {
                    io::Error::new(io::ErrorKind::BrokenPipe, "Output channel closed")
                })?;

                // Pause to allow buffer to fill up a little bit, avoiding
                // spamming with a lot of smaller responses
                tokio::time::sleep(READ_PAUSE_DURATION).await;
            }
            Ok(_) => return Ok(()),
            Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                // Pause to allow buffer to fill up a little bit, avoiding
                // spamming with a lot of smaller responses
                tokio::time::sleep(READ_PAUSE_DURATION).await;
            }
            Err(x) => return Err(x),
        }
    }
}

pub fn spawn_write_task<W>(
    writer: W,
    buf: usize,
) -> (JoinHandle<io::Result<()>>, mpsc::Sender<Vec<u8>>)
where
    W: AsyncWrite + Send + Unpin + 'static,
{
    let (tx, rx) = mpsc::channel(buf);
    let task = tokio::spawn(write_handler(writer, rx));
    (task, tx)
}

/// Continually writes to some writer by reading data from a provided receiver until the receiver
/// or writer is closed
async fn write_handler<W>(mut writer: W, mut channel: mpsc::Receiver<Vec<u8>>) -> io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    while let Some(data) = channel.recv().await {
        writer.write_all(&data).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    //! Tests for `spawn_read_task` and `spawn_write_task` helper functions that bridge
    //! between `AsyncRead`/`AsyncWrite` handles and mpsc channels for process I/O piping.

    use super::*;
    use test_log::test;

    // ---- spawn_read_task ----

    #[test(tokio::test)]
    async fn read_task_should_forward_data_from_reader_to_channel() {
        let data = b"hello world";
        let reader = tokio::io::BufReader::new(&data[..]);

        let (task, mut rx) = spawn_read_task(reader, 10);

        let received = rx.recv().await.unwrap();
        assert_eq!(received, b"hello world");

        // After reader is exhausted, channel should close
        assert!(rx.recv().await.is_none());

        // Task should complete successfully
        task.await.unwrap().unwrap();
    }

    #[test(tokio::test)]
    async fn read_task_should_handle_empty_reader() {
        let data: &[u8] = b"";
        let reader = tokio::io::BufReader::new(data);

        let (task, mut rx) = spawn_read_task(reader, 10);

        // Channel should close immediately since there's no data
        assert!(rx.recv().await.is_none());

        task.await.unwrap().unwrap();
    }

    #[test(tokio::test)]
    async fn read_task_should_handle_large_data_in_chunks() {
        // Create data larger than MAX_PIPE_CHUNK_SIZE
        let data = vec![0xABu8; MAX_PIPE_CHUNK_SIZE * 2 + 100];
        let reader = std::io::Cursor::new(data.clone());

        let (task, mut rx) = spawn_read_task(reader, 10);

        let mut total_received = Vec::new();
        while let Some(chunk) = rx.recv().await {
            total_received.extend_from_slice(&chunk);
        }

        assert_eq!(total_received.len(), data.len());
        assert_eq!(total_received, data);

        task.await.unwrap().unwrap();
    }

    #[test(tokio::test)]
    async fn read_task_should_return_error_if_channel_closed_early() {
        // Use a reader that will produce data, but drop the receiver immediately
        let data = vec![0xCDu8; 1024];
        let reader = std::io::Cursor::new(data);

        let (task, rx) = spawn_read_task(reader, 1);
        drop(rx);

        // The task should eventually return a BrokenPipe error
        let result = task.await.unwrap();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    }

    // ---- spawn_write_task ----

    #[test(tokio::test)]
    async fn write_task_should_forward_data_from_channel_to_writer() {
        let buf = Vec::new();
        let writer = tokio::io::BufWriter::new(buf);

        // Use a duplex stream so we can read back what was written
        let (client, mut server) = tokio::io::duplex(1024);

        let (task, tx) = spawn_write_task(client, 10);

        tx.send(b"hello ".to_vec()).await.unwrap();
        tx.send(b"world".to_vec()).await.unwrap();
        drop(tx);

        task.await.unwrap().unwrap();

        // Read what was written to the server side
        let mut received = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut server, &mut received)
            .await
            .unwrap();
        assert_eq!(received, b"hello world");

        // These variables are leftover scaffolding from initial test setup;
        // they are unused but kept to suppress warnings.
        let _ = buf;
        let _ = writer;
    }

    #[test(tokio::test)]
    async fn write_task_should_complete_ok_when_sender_dropped_immediately() {
        let (client, _server) = tokio::io::duplex(1024);

        let (task, tx) = spawn_write_task(client, 10);
        drop(tx);

        // Task should complete successfully with no data written
        task.await.unwrap().unwrap();
    }

    #[test(tokio::test)]
    async fn write_task_should_handle_multiple_chunks() {
        let (client, mut server) = tokio::io::duplex(4096);

        let (task, tx) = spawn_write_task(client, 10);

        for i in 0..5 {
            tx.send(format!("chunk{i}").into_bytes()).await.unwrap();
        }
        drop(tx);

        task.await.unwrap().unwrap();

        let mut received = Vec::new();
        tokio::io::AsyncReadExt::read_to_end(&mut server, &mut received)
            .await
            .unwrap();
        assert_eq!(
            String::from_utf8(received).unwrap(),
            "chunk0chunk1chunk2chunk3chunk4"
        );
    }

    #[test(tokio::test)]
    async fn read_task_should_handle_wouldblock_errors_gracefully() {
        use std::pin::Pin;
        use std::task::{Context, Poll};

        /// A reader that returns WouldBlock on first read, then returns data, then EOF.
        struct WouldBlockReader {
            state: u8,
        }

        impl AsyncRead for WouldBlockReader {
            fn poll_read(
                mut self: Pin<&mut Self>,
                _cx: &mut Context<'_>,
                buf: &mut tokio::io::ReadBuf<'_>,
            ) -> Poll<io::Result<()>> {
                match self.state {
                    0 => {
                        self.state = 1;
                        Poll::Ready(Err(io::Error::new(io::ErrorKind::WouldBlock, "not ready")))
                    }
                    1 => {
                        self.state = 2;
                        buf.put_slice(b"delayed data");
                        Poll::Ready(Ok(()))
                    }
                    _ => {
                        // EOF
                        Poll::Ready(Ok(()))
                    }
                }
            }
        }

        let reader = WouldBlockReader { state: 0 };
        let (task, mut rx) = spawn_read_task(reader, 10);

        let received = rx.recv().await.unwrap();
        assert_eq!(received, b"delayed data");

        assert!(rx.recv().await.is_none());

        task.await.unwrap().unwrap();
    }
}
