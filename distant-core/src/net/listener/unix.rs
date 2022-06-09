use crate::net::{AcceptFuture, Listener};
use tokio::{
    io,
    net::{UnixListener, UnixStream},
};

impl Listener for UnixListener {
    type Output = UnixStream;

    fn accept<'a>(&'a self) -> AcceptFuture<'a, Self::Output>
    where
        Self: Sync + 'a,
    {
        async fn accept(_self: &UnixListener) -> io::Result<UnixStream> {
            UnixListener::accept(_self).await.map(|(stream, _)| stream)
        }

        Box::pin(accept(self))
    }
}
