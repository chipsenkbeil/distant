use crate::net::{AcceptFuture, Listener};
use tokio::{io, net::windows::named_pipe::NamedPipeServer};

impl Listener for NamedPipeServer {
    type Output = NamedPipeServer;

    fn accept<'a>(&'a self) -> AcceptFuture<'a, Self::Output>
    where
        Self: Sync + 'a,
    {
        async fn accept(_self: &NamedPipeServer) -> io::Result<NamedPipeServer> {
            _self.accept().await.map(|(stream, _)| stream)
        }

        Box::pin(accept(self))
    }
}
