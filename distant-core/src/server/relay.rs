use crate::{
    client::{Session, SessionChannel},
    data::{DistantRequestData, DistantResponseData, Request},
    server::utils::{ConnTracker, ShutdownTask},
};
use distant_net::{Codec, FramedTransport, Transport};
use futures::stream::{Stream, StreamExt};
use log::*;
use std::{collections::HashMap, marker::Unpin, sync::Arc};
use tokio::{
    io,
    sync::{oneshot, Mutex},
    task::{JoinError, JoinHandle},
    time::Duration,
};

/// Represents a server that relays requests & responses between connections and the
/// actual server
pub struct RelayServer {
    accept_task: JoinHandle<()>,
    conns: Arc<Mutex<HashMap<usize, Conn>>>,
}

impl RelayServer {
    pub fn initialize<T, U, S>(
        session: Session,
        mut stream: S,
        shutdown_after: Option<Duration>,
    ) -> io::Result<Self>
    where
        T: Transport + Send + 'static,
        U: Codec + Send + 'static,
        S: Stream<Item = FramedTransport<T, U>> + Send + Unpin + 'static,
    {
        let conns: Arc<Mutex<HashMap<usize, Conn>>> = Arc::new(Mutex::new(HashMap::new()));

        let (shutdown, tracker) = ShutdownTask::maybe_initialize(shutdown_after);
        let conns_2 = Arc::clone(&conns);
        let accept_task = tokio::spawn(async move {
            let inner = async move {
                loop {
                    let channel = session.clone_channel();
                    match stream.next().await {
                        Some(transport) => {
                            let result = Conn::initialize(
                                transport,
                                channel,
                                tracker.as_ref().map(Arc::clone),
                            )
                            .await;

                            match result {
                                Ok(conn) => {
                                    conns_2.lock().await.insert(conn.id(), conn);
                                }
                                Err(x) => {
                                    error!("Failed to initialize connection: {}", x);
                                }
                            };
                        }
                        None => {
                            info!("Listener shutting down");
                            break;
                        }
                    };
                }
            };

            match shutdown {
                Some(shutdown) => tokio::select! {
                    _ = inner => {}
                    _ = shutdown => {
                        warn!("Reached shutdown timeout, so terminating");
                    }
                },
                None => inner.await,
            }
        });

        Ok(Self { accept_task, conns })
    }

    /// Waits for the server to terminate
    pub async fn wait(self) -> Result<(), JoinError> {
        self.accept_task.await
    }

    /// Aborts the server by aborting the internal tasks and current connections
    pub async fn abort(&self) {
        self.accept_task.abort();
        self.conns
            .lock()
            .await
            .values()
            .for_each(|conn| conn.abort());
    }
}

struct Conn {
    id: usize,
    conn_task: JoinHandle<()>,
}

impl Conn {
    pub async fn initialize<T, U>(
        transport: FramedTransport<T, U>,
        channel: SessionChannel,
        ct: Option<Arc<Mutex<ConnTracker>>>,
    ) -> io::Result<Self>
    where
        T: Transport + 'static,
        U: Codec + Send + 'static,
    {
        // Create a unique id to associate with the connection since its address
        // is not guaranteed to have an identifiable string
        let id: usize = rand::random();

        // Mark that we have a new connection
        if let Some(ct) = ct.as_ref() {
            ct.lock().await.increment();
        }

        let conn_task = spawn_conn_handler(id, transport, channel, ct).await;

        Ok(Self { id, conn_task })
    }

    /// Id associated with the connection
    pub fn id(&self) -> usize {
        self.id
    }

    /// Aborts the connection from the server side
    pub fn abort(&self) {
        self.conn_task.abort();
    }
}

async fn spawn_conn_handler<T, U>(
    conn_id: usize,
    transport: FramedTransport<T, U>,
    mut channel: SessionChannel,
    ct: Option<Arc<Mutex<ConnTracker>>>,
) -> JoinHandle<()>
where
    T: Transport,
    U: Codec + Send + 'static,
{
    let (mut t_reader, t_writer) = transport.into_split();
    let processes = Arc::new(Mutex::new(Vec::new()));
    let t_writer = Arc::new(Mutex::new(t_writer));

    let (done_tx, done_rx) = oneshot::channel();
    let mut channel_2 = channel.clone();
    let processes_2 = Arc::clone(&processes);
    let task = tokio::spawn(async move {
        loop {
            if channel_2.is_closed() {
                break;
            }

            // For each request, forward it through the session and monitor all responses
            match t_reader.receive::<Request>().await {
                Ok(Some(req)) => match channel_2.mail(req).await {
                    Ok(mut mailbox) => {
                        let processes = Arc::clone(&processes_2);
                        let t_writer = Arc::clone(&t_writer);
                        tokio::spawn(async move {
                            while let Some(res) = mailbox.next().await {
                                // Keep track of processes that are started so we can kill them
                                // when we're done
                                {
                                    let mut p_lock = processes.lock().await;
                                    for data in res.payload.iter() {
                                        if let DistantResponseData::ProcSpawned { id } = *data {
                                            p_lock.push(id);
                                        }
                                    }
                                }

                                if let Err(x) = t_writer.lock().await.send(res).await {
                                    error!(
                                        "<Conn @ {}> Failed to send response back: {}",
                                        conn_id, x
                                    );
                                }
                            }
                        });
                    }
                    Err(x) => error!(
                        "<Conn @ {}> Failed to pass along request received on unix socket: {:?}",
                        conn_id, x
                    ),
                },
                Ok(None) => break,
                Err(x) => {
                    error!(
                        "<Conn @ {}> Failed to receive request from unix stream: {:?}",
                        conn_id, x
                    );
                    break;
                }
            }
        }

        let _ = done_tx.send(());
    });

    // Perform cleanup if done by sending a request to kill each running process
    tokio::spawn(async move {
        let _ = done_rx.await;

        let p_lock = processes.lock().await;
        if !p_lock.is_empty() {
            trace!(
                "Cleaning conn {} :: killing {} process",
                conn_id,
                p_lock.len()
            );
            if let Err(x) = channel
                .fire(Request::new(
                    "relay",
                    p_lock
                        .iter()
                        .map(|id| DistantRequestData::ProcKill { id: *id })
                        .collect(),
                ))
                .await
            {
                error!("<Conn @ {}> Failed to send kill signals: {}", conn_id, x);
            }
        }

        if let Some(ct) = ct.as_ref() {
            ct.lock().await.decrement();
        }
        debug!("<Conn @ {}> Disconnected", conn_id);
    });

    task
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::Response;
    use distant_net::{InmemoryTransport, PlainCodec};
    use std::{pin::Pin, time::Duration};
    use tokio::sync::mpsc;

    fn make_session() -> (FramedTransport<InmemoryTransport, PlainCodec>, Session) {
        let (t1, t2) = FramedTransport::make_pair();
        (t1, Session::initialize(t2).unwrap())
    }

    #[allow(clippy::type_complexity)]
    fn make_transport_stream() -> (
        mpsc::Sender<FramedTransport<InmemoryTransport, PlainCodec>>,
        Pin<Box<dyn Stream<Item = FramedTransport<InmemoryTransport, PlainCodec>> + Send>>,
    ) {
        let (tx, rx) = mpsc::channel::<FramedTransport<InmemoryTransport, PlainCodec>>(1);
        let stream = futures::stream::unfold(rx, |mut rx| async move {
            rx.recv().await.map(move |transport| (transport, rx))
        });
        (tx, Box::pin(stream))
    }

    #[tokio::test]
    async fn wait_should_return_ok_when_all_inner_tasks_complete() {
        let (transport, session) = make_session();
        let (tx, stream) = make_transport_stream();
        let server = RelayServer::initialize(session, stream, None).unwrap();

        // Conclude all server tasks by closing out the listener & session
        drop(transport);
        drop(tx);

        let result = server.wait().await;
        assert!(result.is_ok(), "Unexpected result: {:?}", result);
    }

    #[tokio::test]
    async fn wait_should_return_error_when_server_aborted() {
        let (_transport, session) = make_session();
        let (_tx, stream) = make_transport_stream();
        let server = RelayServer::initialize(session, stream, None).unwrap();
        server.abort().await;

        match server.wait().await {
            Err(x) if x.is_cancelled() => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[tokio::test]
    async fn server_should_forward_requests_using_session() {
        let (mut transport, session) = make_session();
        let (tx, stream) = make_transport_stream();
        let _server = RelayServer::initialize(session, stream, None).unwrap();

        // Send over a "connection"
        let (mut t1, t2) = FramedTransport::make_pair();
        tx.send(t2).await.unwrap();

        // Send a request
        let req = Request::new("test-tenant", vec![DistantRequestData::SystemInfo {}]);
        t1.send(req.clone()).await.unwrap();

        // Verify the request is forwarded out via session
        let outbound_req = transport.receive().await.unwrap().unwrap();
        assert_eq!(req, outbound_req);
    }

    #[tokio::test]
    async fn server_should_send_back_response_with_tenant_matching_connection() {
        let (mut transport, session) = make_session();
        let (tx, stream) = make_transport_stream();
        let _server = RelayServer::initialize(session, stream, None).unwrap();

        // Send over a "connection"
        let (mut t1, t2) = FramedTransport::make_pair();
        tx.send(t2).await.unwrap();

        // Send over a second "connection"
        let (mut t2, t3) = FramedTransport::make_pair();
        tx.send(t3).await.unwrap();

        // Send a request to mark the tenant of the first connection
        t1.send(Request::new(
            "test-tenant-1",
            vec![DistantRequestData::SystemInfo {}],
        ))
        .await
        .unwrap();

        // Send a request to mark the tenant of the second connection
        t2.send(Request::new(
            "test-tenant-2",
            vec![DistantRequestData::SystemInfo {}],
        ))
        .await
        .unwrap();

        // Clear out the transport channel (outbound of session)
        // NOTE: Because our test stream uses a buffer size of 1, we have to clear out the
        //       outbound data from the earlier requests before we can send back a response
        let req_1 = transport.receive::<Request>().await.unwrap().unwrap();
        let req_2 = transport.receive::<Request>().await.unwrap().unwrap();
        let origin_id = if req_1.tenant == "test-tenant-2" {
            req_1.id
        } else {
            req_2.id
        };

        // Send a response back to a singular connection based on the tenant
        let res = Response::new("test-tenant-2", origin_id, vec![DistantResponseData::Ok]);
        transport.send(res.clone()).await.unwrap();

        // Verify that response is only received by a singular connection
        let inbound_res = t2.receive().await.unwrap().unwrap();
        assert_eq!(res, inbound_res);

        let no_inbound = tokio::select! {
            _ = t1.receive::<Response>() => {false}
            _ = tokio::time::sleep(Duration::from_millis(50)) => {true}
        };
        assert!(no_inbound, "Unexpectedly got response for wrong connection");
    }

    #[tokio::test]
    async fn server_should_shutdown_if_no_connections_after_shutdown_duration() {
        let (_transport, session) = make_session();
        let (_tx, stream) = make_transport_stream();
        let server =
            RelayServer::initialize(session, stream, Some(Duration::from_millis(50))).unwrap();

        let result = server.wait().await;
        assert!(result.is_ok(), "Unexpected result: {:?}", result);
    }
}
