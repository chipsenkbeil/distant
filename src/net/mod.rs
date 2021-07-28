mod transport;
pub use transport::{Transport, TransportError};

use crate::{
    data::{Request, Response, ResponsePayload},
    utils::Session,
};
use log::*;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};
use tokio::{
    io,
    sync::{oneshot, watch},
};
use tokio_stream::wrappers::WatchStream;

type Callbacks = Arc<Mutex<HashMap<usize, oneshot::Sender<Response>>>>;

/// Represents a client that can make requests against a server
pub struct Client {
    /// Underlying transport used by client
    transport: Arc<tokio::sync::Mutex<Transport>>,

    /// Collection of callbacks to be invoked upon receiving a response to a request
    callbacks: Callbacks,

    /// Callback to trigger when a response is received without an origin or with an origin
    /// not found in the list of callbacks
    rx: watch::Receiver<Response>,
}

impl Client {
    /// Establishes a connection using the provided session
    pub async fn connect(session: Session) -> io::Result<Self> {
        let transport = Arc::new(tokio::sync::Mutex::new(Transport::connect(session).await?));
        let callbacks: Callbacks = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = watch::channel(Response::from(ResponsePayload::Error {
            description: String::from("Fake server response"),
        }));

        // Start a task that continually checks for responses and triggers callbacks
        let transport_2 = Arc::clone(&transport);
        let callbacks_2 = Arc::clone(&callbacks);
        tokio::spawn(async move {
            loop {
                match transport_2.lock().await.receive::<Response>().await {
                    Ok(Some(res)) => {
                        let maybe_callback = res
                            .origin_id
                            .as_ref()
                            .and_then(|id| callbacks_2.lock().unwrap().remove(id));

                        // If there is an origin to this response, trigger the callback
                        if let Some(tx) = maybe_callback {
                            if let Err(res) = tx.send(res) {
                                error!("Failed to trigger callback for response {}", res.id);
                            }

                        // Otherwise, this goes into the junk draw of response handlers
                        } else {
                            if let Err(x) = tx.send(res) {
                                error!("Failed to trigger watch: {}", x);
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
            transport,
            callbacks,
            rx,
        })
    }

    /// Sends a request and waits for a response
    pub async fn send(&self, req: Request) -> Result<Response, TransportError> {
        // First, add a callback that will trigger when we get the response for this request
        let (tx, rx) = oneshot::channel();
        self.callbacks.lock().unwrap().insert(req.id, tx);

        // Second, send the request
        self.transport.lock().await.send(req).await?;

        // Third, wait for the response
        rx.await
            .map_err(|x| TransportError::from(io::Error::new(io::ErrorKind::ConnectionAborted, x)))
    }

    /// Creates and returns a new stream of responses that are received with no originating request
    pub fn to_response_stream(&self) -> WatchStream<Response> {
        WatchStream::new(self.rx.clone())
    }
}
