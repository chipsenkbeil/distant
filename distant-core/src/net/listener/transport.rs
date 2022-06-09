use crate::net::{Codec, DataStream, Listener, Transport};
use futures::stream::Stream;
use log::*;
use tokio::{sync::mpsc, task::JoinHandle};

pub struct TransportListener<T, U>
where
    T: DataStream,
    U: Codec,
{
    listen_task: JoinHandle<()>,
    accept_task: JoinHandle<()>,
    rx: mpsc::Receiver<Transport<T, U>>,
}

impl<T, U> TransportListener<T, U>
where
    T: DataStream + Send + 'static,
    U: Codec + Send + 'static,
{
    pub fn initialize<L, F>(listener: L, mut make_transport: F) -> Self
    where
        L: Listener<Output = T> + 'static,
        F: FnMut(T) -> Transport<T, U> + Send + 'static,
    {
        let (stream_tx, mut stream_rx) = mpsc::channel::<T>(1);
        let listen_task = tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok(stream) => {
                        if stream_tx.send(stream).await.is_err() {
                            error!("Listener failed to pass along stream");
                            break;
                        }
                    }
                    Err(x) => {
                        error!("Listener failed to accept stream: {}", x);
                        break;
                    }
                }
            }
        });

        let (tx, rx) = mpsc::channel::<Transport<T, U>>(1);
        let accept_task = tokio::spawn(async move {
            // Check if we have a new connection. If so, wrap it in a transport and forward
            // it along to
            while let Some(stream) = stream_rx.recv().await {
                let transport = make_transport(stream);
                if let Err(x) = tx.send(transport).await {
                    error!("Failed to forward transport: {}", x);
                }
            }
        });

        Self {
            listen_task,
            accept_task,
            rx,
        }
    }

    pub fn abort(&self) {
        self.listen_task.abort();
        self.accept_task.abort();
    }

    /// Waits for the next fully-initialized transport for an incoming stream to be available,
    /// returning none if no longer accepting new connections
    pub async fn accept(&mut self) -> Option<Transport<T, U>> {
        self.rx.recv().await
    }

    /// Converts into a stream of transport-wrapped connections
    pub fn into_stream(self) -> impl Stream<Item = Transport<T, U>> {
        futures::stream::unfold(self, |mut _self| async move {
            _self
                .accept()
                .await
                .map(move |transport| (transport, _self))
        })
    }
}
