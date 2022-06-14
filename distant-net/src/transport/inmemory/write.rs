use crate::RawTransportWrite;
use futures::ready;
use std::{
    fmt,
    future::Future,
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{io::AsyncWrite, sync::mpsc};

/// Write portion of an inmemory channel
pub struct InmemoryTransportWriteHalf {
    tx: Option<mpsc::Sender<Vec<u8>>>,
    task: Option<Pin<Box<dyn Future<Output = io::Result<usize>> + Send + Sync + 'static>>>,
}

impl InmemoryTransportWriteHalf {
    pub fn new(tx: mpsc::Sender<Vec<u8>>) -> Self {
        Self {
            tx: Some(tx),
            task: None,
        }
    }
}

impl fmt::Debug for InmemoryTransportWriteHalf {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InmemoryTransportWrite")
            .field("tx", &self.tx)
            .field(
                "task",
                &if self.tx.is_some() {
                    "Some(...)"
                } else {
                    "None"
                },
            )
            .finish()
    }
}

impl RawTransportWrite for InmemoryTransportWriteHalf {}

impl AsyncWrite for InmemoryTransportWriteHalf {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            match self.task.as_mut() {
                Some(task) => {
                    let res = ready!(task.as_mut().poll(cx));
                    self.task.take();
                    return Poll::Ready(res);
                }
                None => match self.tx.as_mut() {
                    Some(tx) => {
                        let n = buf.len();
                        let tx_2 = tx.clone();
                        let data = buf.to_vec();
                        let task =
                            Box::pin(async move { tx_2.send(data).await.map(|_| n).or(Ok(0)) });
                        self.task.replace(task);
                    }
                    None => return Poll::Ready(Ok(0)),
                },
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.tx.take();
        self.task.take();
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InmemoryTransport, IntoSplit};
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn write_half_should_return_buf_len_if_can_send_immediately() {
        let (_tx, mut rx, transport) = InmemoryTransport::make(1);
        let (mut t_write, _t_read) = transport.into_split();

        // Write that is not waiting should always succeed with full contents
        let n = t_write.write(&[1, 2, 3]).await.expect("Failed to write");
        assert_eq!(n, 3, "Unexpected byte count returned");

        // Verify we actually had the data sent
        let data = rx.try_recv().expect("Failed to recv data");
        assert_eq!(data, &[1, 2, 3]);
    }

    #[tokio::test]
    async fn write_half_should_return_support_eventually_sending_by_retrying_when_not_ready() {
        let (_tx, mut rx, transport) = InmemoryTransport::make(1);
        let (mut t_write, _t_read) = transport.into_split();

        // Queue a write already so that we block on the next one
        let _ = t_write.write(&[1, 2, 3]).await.expect("Failed to write");

        // Verify that the next write is pending
        let f = t_write.write(&[4, 5]);
        tokio::pin!(f);
        match futures::poll!(&mut f) {
            Poll::Pending => {}
            x => panic!("Unexpected poll result: {:?}", x),
        }

        // Consume first batch of data so future of second can continue
        let data = rx.try_recv().expect("Failed to recv data");
        assert_eq!(data, &[1, 2, 3]);

        // Verify that poll now returns success
        match futures::poll!(f) {
            Poll::Ready(Ok(n)) if n == 2 => {}
            x => panic!("Unexpected poll result: {:?}", x),
        }

        // Consume second batch of data
        let data = rx.try_recv().expect("Failed to recv data");
        assert_eq!(data, &[4, 5]);
    }

    #[tokio::test]
    async fn write_half_should_zero_if_inner_channel_closed() {
        let (_tx, rx, transport) = InmemoryTransport::make(1);
        let (mut t_write, _t_read) = transport.into_split();

        // Drop receiving end that transport would talk to
        drop(rx);

        // Channel is dropped, so return 0 to indicate no bytes sent
        let n = t_write.write(&[1, 2, 3]).await.expect("Failed to write");
        assert_eq!(n, 0, "Unexpected byte count returned");
    }
}
