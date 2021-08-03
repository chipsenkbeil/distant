mod transport;
pub use transport::{Transport, TransportError, TransportReadHalf, TransportWriteHalf};

use crate::{
    constants::CLIENT_BROADCAST_CHANNEL_CAPACITY,
    data::{Request, Response},
    session::Session,
};
use log::*;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::{
    io,
    sync::{broadcast, oneshot},
};
use tokio_stream::wrappers::BroadcastStream;

type Callbacks = Arc<Mutex<HashMap<usize, oneshot::Sender<Response>>>>;

/// Represents a client that can make requests against a server
pub struct Client {
    /// Underlying transport used by client
    t_write: TransportWriteHalf,

    /// Collection of callbacks to be invoked upon receiving a response to a request
    callbacks: Callbacks,

    /// Callback to trigger when a response is received without an origin or with an origin
    /// not found in the list of callbacks
    broadcast: broadcast::Sender<Response>,

    /// Represents an initial receiver for broadcasted responses that can capture responses
    /// prior to a stream being established and consumed
    init_broadcast_receiver: Option<broadcast::Receiver<Response>>,
}

impl Client {
    /// Establishes a connection using the provided session
    pub async fn connect(session: Session) -> io::Result<Self> {
        let transport = Transport::connect(session).await?;
        debug!(
            "Client has connected to {}",
            transport
                .peer_addr()
                .map(|x| x.to_string())
                .unwrap_or_else(|_| String::from("???"))
        );

        let (mut t_read, t_write) = transport.into_split();
        let callbacks: Callbacks = Arc::new(Mutex::new(HashMap::new()));
        let (broadcast, init_broadcast_receiver) =
            broadcast::channel(CLIENT_BROADCAST_CHANNEL_CAPACITY);

        // Start a task that continually checks for responses and triggers callbacks
        let callbacks_2 = Arc::clone(&callbacks);
        let broadcast_2 = broadcast.clone();
        tokio::spawn(async move {
            loop {
                match t_read.receive::<Response>().await {
                    Ok(Some(res)) => {
                        trace!("Client got response: {:?}", res);
                        let maybe_callback = res
                            .origin_id
                            .as_ref()
                            .and_then(|id| callbacks_2.lock().unwrap().remove(id));

                        // If there is an origin to this response, trigger the callback
                        if let Some(tx) = maybe_callback {
                            trace!("Client has callback! Triggering!");
                            if let Err(res) = tx.send(res) {
                                error!("Failed to trigger callback for response {}", res.id);
                            }

                        // Otherwise, this goes into the junk draw of response handlers
                        } else {
                            trace!("Client does not have callback! Broadcasting!");
                            if let Err(x) = broadcast_2.send(res) {
                                error!("Failed to trigger broadcast: {}", x);
                            }
                        }
                    }
                    Ok(None) => break,
                    Err(x) => {
                        error!("{}", x);
                        break;
                    }
                }
            }
        });

        Ok(Self {
            t_write,
            callbacks,
            broadcast,
            init_broadcast_receiver: Some(init_broadcast_receiver),
        })
    }

    /// Sends a request and waits for a response
    pub async fn send(&mut self, req: Request) -> Result<Response, TransportError> {
        // First, add a callback that will trigger when we get the response for this request
        let (tx, rx) = oneshot::channel();
        self.callbacks.lock().unwrap().insert(req.id, tx);

        // Second, send the request
        self.t_write.send(req).await?;

        // Third, wait for the response
        rx.await
            .map_err(|x| TransportError::from(io::Error::new(io::ErrorKind::ConnectionAborted, x)))
    }

    /// Creates and returns a new stream of responses that are received with no originating request
    pub fn to_response_stream(&mut self) -> BroadcastStream<Response> {
        BroadcastStream::new(
            self.init_broadcast_receiver
                .take()
                .unwrap_or_else(|| self.broadcast.subscribe()),
        )
    }
}
