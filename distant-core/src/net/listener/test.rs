use crate::net::{AcceptFuture, DataStream, Listener};
use tokio::{
    io,
    sync::{mpsc, Mutex},
};

impl<T> Listener for Mutex<mpsc::Receiver<T>>
where
    T: DataStream + Send + Sync + 'static,
{
    type Output = T;

    fn accept<'a>(&'a self) -> AcceptFuture<'a, Self::Output>
    where
        Self: Sync + 'a,
    {
        async fn accept<T>(_self: &Mutex<mpsc::Receiver<T>>) -> io::Result<T>
        where
            T: DataStream + Send + Sync + 'static,
        {
            _self
                .lock()
                .await
                .recv()
                .await
                .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))
        }

        Box::pin(accept(self))
    }
}
