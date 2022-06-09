use crate::net::{AcceptFuture, Listener};
use tokio::{
    io,
    net::{TcpListener as TokioTcpListener, TcpStream as TokioTcpStream},
};

pub struct TcpListener(TokioTcpListener);

impl Listener for TcpListener {
    type Output = TcpStream;

    fn accept<'a>(&'a self) -> AcceptFuture<'a, Self::Output>
    where
        Self: Sync + 'a,
    {
        async fn accept(_self: &TcpListener) -> io::Result<TcpStream> {
            TcpListener::accept(_self).await.map(|(stream, _)| stream)
        }

        Box::pin(accept(self))
    }
}
