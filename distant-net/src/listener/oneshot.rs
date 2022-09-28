use crate::Listener;
use async_trait::async_trait;
use derive_more::From;
use std::io;
use tokio::sync::oneshot;

/// Represents a [`Listener`] that only has a single connection
#[derive(From)]
pub struct OneshotListener<T: Send> {
    inner: Option<oneshot::Receiver<T>>,
}

impl<T: Send> OneshotListener<T> {
    pub fn from_value(value: T) -> Self {
        let (tx, listener) = Self::channel();

        // NOTE: Impossible to fail as the receiver has not been dropped at this point
        let _ = tx.send(value);

        listener
    }

    pub fn channel() -> (oneshot::Sender<T>, Self) {
        let (tx, rx) = oneshot::channel();
        (tx, Self { inner: Some(rx) })
    }
}

#[async_trait]
impl<T: Send> Listener for OneshotListener<T> {
    type Output = T;

    /// First call to accept will return listener tied to [`OneshotListener`] while future
    /// calls will yield an error of `io::ErrorKind::ConnectionAborted`
    async fn accept(&mut self) -> io::Result<Self::Output> {
        match self.inner.take() {
            Some(rx) => rx
                .await
                .map_err(|x| io::Error::new(io::ErrorKind::BrokenPipe, x)),
            None => Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "Oneshot listener has concluded",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;
    use tokio::task::JoinHandle;

    #[test(tokio::test)]
    async fn from_value_should_return_value_on_first_call_to_accept() {
        let mut listener = OneshotListener::from_value("hello world");
        assert_eq!(listener.accept().await.unwrap(), "hello world");
        assert_eq!(
            listener.accept().await.unwrap_err().kind(),
            io::ErrorKind::ConnectionAborted
        );
    }

    #[test(tokio::test)]
    async fn channel_should_return_a_oneshot_sender_to_feed_first_call_to_accept() {
        let (tx, mut listener) = OneshotListener::channel();
        let accept_task: JoinHandle<(io::Result<&str>, io::Result<&str>)> =
            tokio::spawn(async move {
                let result_1 = listener.accept().await;
                let result_2 = listener.accept().await;
                (result_1, result_2)
            });
        tokio::spawn(async move {
            tx.send("hello world").unwrap();
        });

        let (result_1, result_2) = accept_task.await.unwrap();

        assert_eq!(result_1.unwrap(), "hello world");
        assert_eq!(
            result_2.unwrap_err().kind(),
            io::ErrorKind::ConnectionAborted
        );
    }
}
