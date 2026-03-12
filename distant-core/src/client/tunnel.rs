use std::fmt;
use std::io;

use log::*;
use tokio::sync::mpsc;
use tokio::sync::mpsc::error::{TryRecvError, TrySendError};
use tokio::task::JoinHandle;

use crate::client::Channel;
use crate::constants::CLIENT_TUNNEL_CAPACITY;
use crate::net::client::Mailbox;
use crate::net::common::{Request, Response};
use crate::protocol::{self, TunnelId};

/// Represents a bidirectional TCP tunnel to a remote host.
///
/// A `RemoteTunnel` provides a channel for sending and receiving raw bytes
/// through a tunnel established on the remote server. The tunnel connects
/// to a specified host and port on the remote side, forwarding data
/// bidirectionally between the client and the remote endpoint.
///
/// # Lifecycle
///
/// 1. Open the tunnel via [`RemoteTunnel::open`] or [`ChannelExt::tunnel_open`].
/// 2. Take the [`RemoteTunnelWriter`] and [`RemoteTunnelReader`] to send and
///    receive data.
/// 3. Close the tunnel when finished via [`RemoteTunnel::close`], or abort it
///    via [`RemoteTunnel::abort`].
///
/// [`ChannelExt::tunnel_open`]: crate::client::ChannelExt::tunnel_open
pub struct RemoteTunnel {
    /// Unique identifier for this tunnel.
    id: TunnelId,

    /// Id used to map back to the mailbox.
    origin_id: String,

    /// Sender to abort the outgoing request task.
    abort_req_task_tx: mpsc::Sender<()>,

    /// Sender to abort the incoming response task.
    abort_res_task_tx: mpsc::Sender<()>,

    /// Writer for sending data through the tunnel.
    pub writer: Option<RemoteTunnelWriter>,

    /// Reader for receiving data from the tunnel.
    pub reader: Option<RemoteTunnelReader>,

    /// Handle for closing the tunnel.
    closer: RemoteTunnelCloser,

    /// Background task that waits for the request and response tasks to complete.
    task: JoinHandle<()>,
}

impl fmt::Debug for RemoteTunnel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteTunnel")
            .field("id", &self.id)
            .field("origin_id", &self.origin_id)
            .finish()
    }
}

impl RemoteTunnel {
    /// Opens a forward tunnel to the specified host and port on the remote server.
    ///
    /// Sends a `TunnelOpen` request via the given channel and waits for a
    /// `TunnelOpened` confirmation. Once confirmed, spawns background tasks
    /// to handle bidirectional data flow.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the server returns an error, sends an unexpected
    /// response, or the connection is lost before confirmation.
    pub async fn open(mut channel: Channel, host: String, port: u16) -> io::Result<Self> {
        trace!("Opening tunnel to {host}:{port}");

        // Submit the open request and get a mailbox for responses
        let mut mailbox = channel
            .mail(Request::new(protocol::Msg::Single(
                protocol::Request::TunnelOpen { host, port },
            )))
            .await?;

        // Wait for the first response: TunnelOpened or Error
        let (id, origin_id) = match mailbox.next().await {
            Some(res) => {
                let origin_id = res.origin_id;
                match res.payload {
                    protocol::Msg::Single(protocol::Response::TunnelOpened { id }) => {
                        (id, origin_id)
                    }
                    protocol::Msg::Single(protocol::Response::Error(x)) => return Err(x.into()),
                    protocol::Msg::Single(x) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("Got response type of {}", x.as_ref()),
                        ));
                    }
                    protocol::Msg::Batch(_) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "Got batch instead of single response",
                        ));
                    }
                }
            }
            None => return Err(io::Error::from(io::ErrorKind::ConnectionAborted)),
        };

        trace!("[Tunnel {id}] Tunnel opened");

        // Create channels for writer and reader
        let (writer_tx, writer_rx) = mpsc::channel(CLIENT_TUNNEL_CAPACITY);
        let (reader_tx, reader_rx) = mpsc::channel(CLIENT_TUNNEL_CAPACITY);

        // Close channel: used to signal a graceful close
        let (close_tx, close_rx) = mpsc::channel(1);
        let close_tx_2 = close_tx.clone();

        // Spawn the response task: reads from the mailbox and routes data
        let (abort_res_task_tx, mut abort_res_task_rx) = mpsc::channel::<()>(1);
        let res_task = tokio::spawn(async move {
            tokio::select! {
                _ = abort_res_task_rx.recv() => {
                    // Abort signal received; panic is caught by tokio::spawn
                    // as a JoinError and does not crash the process.
                    panic!("tunnel response task aborted");
                }
                res = process_tunnel_incoming(id, mailbox, reader_tx, close_tx_2) => {
                    res
                }
            }
        });

        // Spawn the request task: reads from writer rx and close rx, sends to server
        let (abort_req_task_tx, mut abort_req_task_rx) = mpsc::channel::<()>(1);
        let req_task = tokio::spawn(async move {
            tokio::select! {
                _ = abort_req_task_rx.recv() => {
                    // Abort signal received; panic is caught by tokio::spawn
                    // as a JoinError and does not crash the process.
                    panic!("tunnel request task aborted");
                }
                res = process_tunnel_outgoing(id, channel, writer_rx, close_rx) => {
                    res
                }
            }
        });

        // Spawn a wait task that joins both
        let wait_task = tokio::spawn(async move {
            let _ = tokio::try_join!(req_task, res_task);
        });

        Ok(Self {
            id,
            origin_id,
            abort_req_task_tx,
            abort_res_task_tx,
            writer: Some(RemoteTunnelWriter(writer_tx)),
            reader: Some(RemoteTunnelReader(reader_rx)),
            closer: RemoteTunnelCloser(close_tx),
            task: wait_task,
        })
    }

    /// Returns the unique identifier for this tunnel.
    pub fn id(&self) -> TunnelId {
        self.id
    }

    /// Returns the origin request id that opened this tunnel.
    pub fn origin_id(&self) -> &str {
        &self.origin_id
    }

    /// Sends a close signal to gracefully shut down the tunnel.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the close signal could not be sent (e.g., the tunnel
    /// is already closed).
    pub async fn close(&mut self) -> io::Result<()> {
        self.closer.close().await
    }

    /// Aborts the tunnel by forcing both background tasks to shut down.
    ///
    /// Unlike [`close`](Self::close), this does not send a `TunnelClose`
    /// request to the server.
    pub fn abort(&self) {
        let _ = self.abort_req_task_tx.try_send(());
        let _ = self.abort_res_task_tx.try_send(());
    }

    /// Waits for the tunnel's background tasks to complete, consuming the tunnel.
    pub async fn wait(self) {
        let _ = self.task.await;
    }

    /// Returns true if the tunnel's background task is still running.
    pub fn is_active(&self) -> bool {
        !self.task.is_finished()
    }
}

/// Writer half for sending data through a [`RemoteTunnel`].
///
/// Obtained via [`RemoteTunnel::writer`]. Queues outgoing data for
/// transmission through the tunnel's underlying channel.
#[derive(Clone, Debug)]
pub struct RemoteTunnelWriter(mpsc::Sender<Vec<u8>>);

impl RemoteTunnelWriter {
    /// Sends data through the tunnel, waiting if the internal buffer is full.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::BrokenPipe`] if the tunnel has been closed.
    pub async fn write(&mut self, data: impl Into<Vec<u8>>) -> io::Result<()> {
        self.0
            .send(data.into())
            .await
            .map_err(|x| io::Error::new(io::ErrorKind::BrokenPipe, x))
    }

    /// Tries to send data through the tunnel without waiting.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::WouldBlock`] if the internal buffer is full,
    /// or [`io::ErrorKind::BrokenPipe`] if the tunnel has been closed.
    pub fn try_write(&mut self, data: impl Into<Vec<u8>>) -> io::Result<()> {
        match self.0.try_send(data.into()) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(io::Error::from(io::ErrorKind::WouldBlock)),
            Err(TrySendError::Closed(_)) => Err(io::Error::from(io::ErrorKind::BrokenPipe)),
        }
    }

    /// Returns true if the tunnel writer has been closed.
    pub fn is_closed(&self) -> bool {
        self.0.is_closed()
    }
}

/// Reader half for receiving data from a [`RemoteTunnel`].
///
/// Obtained via [`RemoteTunnel::reader`]. Yields data chunks arriving
/// through the tunnel from the remote side.
#[derive(Debug)]
pub struct RemoteTunnelReader(mpsc::Receiver<Vec<u8>>);

impl RemoteTunnelReader {
    /// Receives the next chunk of data from the tunnel, waiting until data is available.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::BrokenPipe`] if the tunnel has been closed and
    /// no more data is available.
    pub async fn read(&mut self) -> io::Result<Vec<u8>> {
        self.0
            .recv()
            .await
            .ok_or_else(|| io::Error::from(io::ErrorKind::BrokenPipe))
    }

    /// Tries to receive data from the tunnel without waiting.
    ///
    /// Returns `Ok(None)` if no data is currently available, `Ok(Some(data))`
    /// if data was received.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::BrokenPipe`] if the tunnel has been closed
    /// and no more data is available.
    pub fn try_read(&mut self) -> io::Result<Option<Vec<u8>>> {
        match self.0.try_recv() {
            Ok(data) => Ok(Some(data)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(io::Error::from(io::ErrorKind::BrokenPipe)),
        }
    }
}

/// Handle for closing a [`RemoteTunnel`].
///
/// Can be cloned to allow closing the tunnel from multiple locations.
#[derive(Clone, Debug)]
pub struct RemoteTunnelCloser(mpsc::Sender<()>);

impl RemoteTunnelCloser {
    /// Sends a close signal to gracefully shut down the tunnel.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::BrokenPipe`] if the tunnel is already closed.
    pub async fn close(&mut self) -> io::Result<()> {
        self.0
            .send(())
            .await
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "Tunnel is already closed"))
    }
}

/// Notification of a new incoming connection on a reverse tunnel listener.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncomingTunnel {
    /// Unique identifier for the new tunnel.
    pub tunnel_id: TunnelId,
    /// Peer address of the incoming connection, if available.
    pub peer_addr: Option<String>,
}

/// Represents a reverse tunnel listener on the remote server.
///
/// A `RemoteTunnelListener` listens on a specified host and port on the remote
/// side, notifying the client of incoming connections via [`IncomingTunnel`]
/// events.
///
/// # Lifecycle
///
/// 1. Start listening via [`RemoteTunnelListener::listen`] or
///    [`ChannelExt::tunnel_listen`].
/// 2. Receive incoming connections via [`next`](Self::next).
/// 3. Close the listener when finished via [`close`](Self::close).
///
/// [`ChannelExt::tunnel_listen`]: crate::client::ChannelExt::tunnel_listen
pub struct RemoteTunnelListener {
    /// Channel used to send the close request.
    channel: Channel,

    /// Unique identifier for this listener.
    id: TunnelId,

    /// Actual port the listener is bound to.
    port: u16,

    /// Background task that processes incoming connection events.
    task: JoinHandle<()>,

    /// Receiver for incoming tunnel notifications.
    rx: mpsc::Receiver<IncomingTunnel>,
}

impl fmt::Debug for RemoteTunnelListener {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RemoteTunnelListener")
            .field("id", &self.id)
            .field("port", &self.port)
            .finish()
    }
}

impl RemoteTunnelListener {
    /// Starts a reverse tunnel listener on the specified host and port on the
    /// remote server.
    ///
    /// Sends a `TunnelListen` request and waits for a `TunnelListening`
    /// confirmation. Once confirmed, spawns a background task to process
    /// incoming connection notifications.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the server returns an error, sends an unexpected
    /// response, or the connection is lost before confirmation.
    pub async fn listen(mut channel: Channel, host: String, port: u16) -> io::Result<Self> {
        trace!("Starting tunnel listener on {host}:{port}");

        // Submit the listen request and get a mailbox for responses
        let mut mailbox = channel
            .mail(Request::new(protocol::Msg::Single(
                protocol::Request::TunnelListen { host, port },
            )))
            .await?;

        // Wait for the first response: TunnelListening or Error
        let (id, actual_port) = match mailbox.next().await {
            Some(res) => match res.payload {
                protocol::Msg::Single(protocol::Response::TunnelListening { id, port }) => {
                    (id, port)
                }
                protocol::Msg::Single(protocol::Response::Error(x)) => return Err(x.into()),
                protocol::Msg::Single(x) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("Got response type of {}", x.as_ref()),
                    ));
                }
                protocol::Msg::Batch(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Got batch instead of single response",
                    ));
                }
            },
            None => return Err(io::Error::from(io::ErrorKind::ConnectionAborted)),
        };

        trace!("[Listener {id}] Listening on port {actual_port}");

        let (tx, rx) = mpsc::channel(CLIENT_TUNNEL_CAPACITY);

        // Spawn a task that processes incoming connection notifications
        let task = tokio::spawn(async move {
            while let Some(res) = mailbox.next().await {
                let mut done = false;

                for data in res.payload.into_vec() {
                    match data {
                        protocol::Response::TunnelIncoming {
                            listener_id,
                            tunnel_id,
                            peer_addr,
                        } if listener_id == id => {
                            if tx.is_closed() {
                                break;
                            }

                            let incoming = IncomingTunnel {
                                tunnel_id,
                                peer_addr,
                            };

                            if let Err(x) = tx.send(incoming).await {
                                error!("[Listener {id}] Failed to send incoming tunnel {:?}", x.0);
                                break;
                            }
                        }

                        protocol::Response::TunnelClosed { id: closed_id } if closed_id == id => {
                            trace!("[Listener {id}] Listener has been closed");
                            done = true;
                            break;
                        }

                        _ => continue,
                    }
                }

                if done {
                    break;
                }
            }
        });

        Ok(Self {
            channel,
            id,
            port: actual_port,
            task,
            rx,
        })
    }

    /// Returns the unique identifier for this listener.
    pub fn id(&self) -> TunnelId {
        self.id
    }

    /// Returns the actual port the listener is bound to.
    ///
    /// This may differ from the requested port if port 0 was specified (OS-assigned
    /// ephemeral port).
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Returns true if the listener's background task is still running.
    pub fn is_active(&self) -> bool {
        !self.task.is_finished()
    }

    /// Receives the next incoming tunnel connection, or `None` if the listener
    /// has been closed.
    pub async fn next(&mut self) -> Option<IncomingTunnel> {
        self.rx.recv().await
    }

    /// Closes the listener by sending a `TunnelClose` request to the server
    /// and aborting the background task.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the close request could not be sent.
    pub async fn close(&mut self) -> io::Result<()> {
        trace!("[Listener {}] Closing listener", self.id);
        self.channel
            .fire(Request::new(protocol::Msg::Single(
                protocol::Request::TunnelClose { id: self.id },
            )))
            .await?;
        self.task.abort();
        Ok(())
    }
}

/// Processes incoming responses from the mailbox for a forward tunnel, routing
/// `TunnelData` to the reader and handling `TunnelClosed` to stop the loop.
async fn process_tunnel_incoming(
    tunnel_id: TunnelId,
    mut mailbox: Mailbox<Response<protocol::Msg<protocol::Response>>>,
    reader_tx: mpsc::Sender<Vec<u8>>,
    close_tx: mpsc::Sender<()>,
) -> io::Result<()> {
    while let Some(res) = mailbox.next().await {
        let payload = res.payload.into_vec();

        // Check if any of the payload data is a close notification
        let is_closed = payload.iter().any(
            |data| matches!(data, protocol::Response::TunnelClosed { id } if *id == tunnel_id),
        );

        // Route tunnel data to the reader
        for data in payload {
            match data {
                protocol::Response::TunnelData { id, data } if id == tunnel_id => {
                    let _ = reader_tx.send(data).await;
                }
                _ => {}
            }
        }

        // If we received a close notification, signal the request task and exit
        if is_closed {
            trace!("[Tunnel {tunnel_id}] Received TunnelClosed");
            let _ = close_tx.try_send(());
            return Ok(());
        }
    }

    // Signal the request task that we're done
    let _ = close_tx.try_send(());

    trace!("[Tunnel {tunnel_id}] Tunnel incoming channel closed");
    Err(io::Error::from(io::ErrorKind::UnexpectedEof))
}

/// Processes outgoing requests for a forward tunnel, reading from the writer
/// channel and close channel, and sending data or close requests to the server.
async fn process_tunnel_outgoing(
    tunnel_id: TunnelId,
    mut channel: Channel,
    mut writer_rx: mpsc::Receiver<Vec<u8>>,
    mut close_rx: mpsc::Receiver<()>,
) -> io::Result<()> {
    let result = loop {
        tokio::select! {
            data = writer_rx.recv() => {
                match data {
                    Some(data) => channel.fire(
                        Request::new(
                            protocol::Msg::Single(protocol::Request::TunnelWrite {
                                id: tunnel_id,
                                data,
                            })
                        )
                    ).await?,
                    None => break Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "Channel is dead",
                    )),
                }
            }
            msg = close_rx.recv() => {
                if msg.is_some() {
                    channel.fire(Request::new(
                        protocol::Msg::Single(protocol::Request::TunnelClose {
                            id: tunnel_id,
                        })
                    )).await?;
                    break Ok(());
                } else {
                    break Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "Channel is dead",
                    ));
                }
            }
        }
    };

    trace!("[Tunnel {tunnel_id}] Tunnel outgoing channel closed");
    result
}
