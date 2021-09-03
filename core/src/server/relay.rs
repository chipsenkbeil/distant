use crate::{
    client::Session,
    constants::CLIENT_BROADCAST_CHANNEL_CAPACITY,
    data::{Request, RequestData, Response, ResponseData},
    net::{DataStream, Transport, TransportReadHalf, TransportWriteHalf},
    server::utils::{ConnTracker, ShutdownTask},
};
use futures::stream::{Stream, StreamExt};
use log::*;
use std::{collections::HashMap, marker::Unpin, sync::Arc};
use tokio::{
    io::{self, AsyncRead, AsyncWrite},
    sync::{mpsc, oneshot, Mutex},
    task::{JoinError, JoinHandle},
    time::Duration,
};

/// Represents a server that relays requests & responses between connections and the
/// actual server
pub struct RelayServer {
    accept_task: JoinHandle<()>,
    broadcast_task: JoinHandle<()>,
    forward_task: JoinHandle<()>,
    conns: Arc<Mutex<HashMap<usize, Conn>>>,
}

impl RelayServer {
    pub fn initialize<T1, T2, S>(
        mut session: Session<T1>,
        mut stream: S,
        shutdown_after: Option<Duration>,
    ) -> io::Result<Self>
    where
        T1: DataStream + 'static,
        T2: DataStream + Send + 'static,
        S: Stream<Item = Transport<T2>> + Send + Unpin + 'static,
    {
        let conns: Arc<Mutex<HashMap<usize, Conn>>> = Arc::new(Mutex::new(HashMap::new()));

        // Spawn task to send server responses to the appropriate connections
        let conns_2 = Arc::clone(&conns);
        debug!("Spawning response broadcast task");
        let mut broadcast = session.broadcast.take().unwrap();
        let (shutdown_broadcast_tx, mut shutdown_broadcast_rx) = mpsc::channel::<()>(1);
        let broadcast_task = tokio::spawn(async move {
            loop {
                let res = tokio::select! {
                    maybe_res = broadcast.recv() => {
                        match maybe_res {
                            Some(res) => res,
                            None => break,
                        }
                    }
                    _ = shutdown_broadcast_rx.recv() => {
                        break;
                    }
                };

                // Search for all connections with a tenant that matches the response's tenant
                for conn in conns_2.lock().await.values_mut() {
                    if conn.state.lock().await.tenant.as_deref() == Some(res.tenant.as_str()) {
                        debug!(
                            "Forwarding response of type{} {} to connection {}",
                            if res.payload.len() > 1 { "s" } else { "" },
                            res.to_payload_type_string(),
                            conn.id
                        );
                        if let Err(x) = conn.forward_response(res).await {
                            error!("Failed to pass forwarding message: {}", x);
                        }

                        // NOTE: We assume that tenant is unique, so we can break after
                        //       forwarding the message to the first matching tenant
                        break;
                    }
                }
            }
        });

        // Spawn task to send to the server requests from connections
        debug!("Spawning request forwarding task");
        let (req_tx, mut req_rx) = mpsc::channel::<Request>(CLIENT_BROADCAST_CHANNEL_CAPACITY);
        let (shutdown_forward_tx, mut shutdown_forward_rx) = mpsc::channel::<()>(1);
        let forward_task = tokio::spawn(async move {
            loop {
                let req = tokio::select! {
                    maybe_req = req_rx.recv() => {
                        match maybe_req {
                            Some(req) => req,
                            None => break,
                        }
                    }
                    _ = shutdown_forward_rx.recv() => {
                        break;
                    }
                };

                debug!(
                    "Forwarding request of type{} {} to server",
                    if req.payload.len() > 1 { "s" } else { "" },
                    req.to_payload_type_string()
                );
                if let Err(x) = session.fire(req).await {
                    error!("Session failed to send request: {:?}", x);
                    break;
                }
            }
        });

        let (shutdown, tracker) = ShutdownTask::maybe_initialize(shutdown_after);
        let conns_2 = Arc::clone(&conns);
        let accept_task = tokio::spawn(async move {
            let inner = async move {
                loop {
                    match stream.next().await {
                        Some(transport) => {
                            let result = Conn::initialize(
                                transport,
                                req_tx.clone(),
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

            // Doesn't matter if we send or drop these as long as they persist until this
            // task is completed, so just drop
            drop(shutdown_broadcast_tx);
            drop(shutdown_forward_tx);
        });

        Ok(Self {
            accept_task,
            broadcast_task,
            forward_task,
            conns,
        })
    }

    /// Waits for the server to terminate
    pub async fn wait(self) -> Result<(), JoinError> {
        match tokio::try_join!(self.accept_task, self.broadcast_task, self.forward_task) {
            Ok(_) => Ok(()),
            Err(x) => Err(x),
        }
    }

    /// Aborts the server by aborting the internal tasks and current connections
    pub async fn abort(&self) {
        self.accept_task.abort();
        self.broadcast_task.abort();
        self.forward_task.abort();
        self.conns
            .lock()
            .await
            .values()
            .for_each(|conn| conn.abort());
    }
}

struct Conn {
    id: usize,
    req_task: JoinHandle<()>,
    res_task: JoinHandle<()>,
    _cleanup_task: JoinHandle<()>,
    res_tx: mpsc::Sender<Response>,
    state: Arc<Mutex<ConnState>>,
}

/// Represents state associated with a connection
#[derive(Default)]
struct ConnState {
    tenant: Option<String>,
    processes: Vec<usize>,
}

impl Conn {
    pub async fn initialize<T>(
        transport: Transport<T>,
        req_tx: mpsc::Sender<Request>,
        ct: Option<Arc<Mutex<ConnTracker>>>,
    ) -> io::Result<Self>
    where
        T: DataStream + 'static,
    {
        // Create a unique id to associate with the connection since its address
        // is not guaranteed to have an identifiable string
        let id: usize = rand::random();

        let (t_read, t_write) = transport.into_split();

        // Used to alert our response task of the connection's tenant name
        // based on the first
        let (tenant_tx, tenant_rx) = oneshot::channel();

        // Create a state we use to keep track of connection-specific data
        debug!("<Conn @ {}> Initializing internal state", id);
        let state = Arc::new(Mutex::new(ConnState::default()));

        // Mark that we have a new connection
        if let Some(ct) = ct.as_ref() {
            ct.lock().await.increment();
        }

        // Spawn task to continually receive responses from the session that
        // may or may not be relevant to the connection, which will filter
        // by tenant and then along any response that matches
        let (res_tx, res_rx) = mpsc::channel::<Response>(CLIENT_BROADCAST_CHANNEL_CAPACITY);
        let (res_task_tx, res_task_rx) = oneshot::channel();
        let state_2 = Arc::clone(&state);
        let res_task = tokio::spawn(async move {
            handle_conn_outgoing(id, state_2, t_write, tenant_rx, res_rx).await;
            let _ = res_task_tx.send(());
        });

        // Spawn task to continually read requests from connection and forward
        // them along to be sent via the session
        let req_tx = req_tx.clone();
        let (req_task_tx, req_task_rx) = oneshot::channel();
        let state_2 = Arc::clone(&state);
        let req_task = tokio::spawn(async move {
            handle_conn_incoming(id, state_2, t_read, tenant_tx, req_tx).await;
            let _ = req_task_tx.send(());
        });

        let _cleanup_task = tokio::spawn(async move {
            let _ = tokio::join!(req_task_rx, res_task_rx);

            if let Some(ct) = ct.as_ref() {
                ct.lock().await.decrement();
            }
            debug!("<Conn @ {}> Disconnected", id);
        });

        Ok(Self {
            id,
            req_task,
            res_task,
            _cleanup_task,
            res_tx,
            state,
        })
    }

    /// Id associated with the connection
    pub fn id(&self) -> usize {
        self.id
    }

    /// Aborts the connection from the server side
    pub fn abort(&self) {
        // NOTE: We don't abort the cleanup task as that needs to actually happen
        //       and will even if these tasks are aborted
        self.req_task.abort();
        self.res_task.abort();
    }

    /// Forwards a response back through this connection
    pub async fn forward_response(
        &mut self,
        res: Response,
    ) -> Result<(), mpsc::error::SendError<Response>> {
        self.res_tx.send(res).await
    }
}

/// Conn::Request -> Session::Fire
async fn handle_conn_incoming<T>(
    conn_id: usize,
    state: Arc<Mutex<ConnState>>,
    mut reader: TransportReadHalf<T>,
    tenant_tx: oneshot::Sender<String>,
    req_tx: mpsc::Sender<Request>,
) where
    T: AsyncRead + Unpin,
{
    macro_rules! process_req {
        ($on_success:expr; $done:expr) => {
            match reader.receive::<Request>().await {
                Ok(Some(req)) => {
                    $on_success(&req);
                    if let Err(x) = req_tx.send(req).await {
                        error!(
                            "Failed to pass along request received on unix socket: {:?}",
                            x
                        );
                        $done;
                    }
                }
                Ok(None) => $done,
                Err(x) => {
                    error!("Failed to receive request from unix stream: {:?}", x);
                    $done;
                }
            }
        };
    }

    let mut tenant = None;

    // NOTE: Have to acquire our first request outside our loop since the oneshot
    //       sender of the tenant's name is consuming
    process_req!(
        |req: &Request| {
            tenant = Some(req.tenant.clone());
            if let Err(x) = tenant_tx.send(req.tenant.clone()) {
                error!("Failed to send along acquired tenant name: {:?}", x);
                return;
            }
        };
        return
    );

    // Loop and process all additional requests
    loop {
        process_req!(|_| {}; break);
    }

    // At this point, we have processed at least one request successfully
    // and should have the tenant populated. If we had a failure at the
    // beginning, we exit the function early via return.
    let tenant = tenant.unwrap();

    // Perform cleanup if done by sending a request to kill each running process
    // debug!("Cleaning conn {} :: killing process {}", conn_id, id);
    if let Err(x) = req_tx
        .send(Request::new(
            tenant.clone(),
            state
                .lock()
                .await
                .processes
                .iter()
                .map(|id| RequestData::ProcKill { id: *id })
                .collect(),
        ))
        .await
    {
        error!("<Conn @ {}> Failed to send kill signals: {}", conn_id, x);
    }
}

async fn handle_conn_outgoing<T>(
    conn_id: usize,
    state: Arc<Mutex<ConnState>>,
    mut writer: TransportWriteHalf<T>,
    tenant_rx: oneshot::Receiver<String>,
    mut res_rx: mpsc::Receiver<Response>,
) where
    T: AsyncWrite + Unpin,
{
    // We wait for the tenant to be identified by the first request
    // before processing responses to be sent back; this is easier
    // to implement and yields the same result as we would be dropping
    // all responses before we know the tenant
    if let Ok(tenant) = tenant_rx.await {
        debug!("Associated tenant {} with conn {}", tenant, conn_id);
        state.lock().await.tenant = Some(tenant.clone());

        while let Some(res) = res_rx.recv().await {
            debug!(
                "Conn {} being sent response of type{} {}",
                conn_id,
                if res.payload.len() > 1 { "s" } else { "" },
                res.to_payload_type_string(),
            );

            // If a new process was started, we want to capture the id and
            // associate it with the connection
            let ids = res.payload.iter().filter_map(|x| match x {
                ResponseData::ProcStart { id } => Some(*id),
                _ => None,
            });
            for id in ids {
                debug!("Tracking proc {} for conn {}", id, conn_id);
                state.lock().await.processes.push(id);
            }

            if let Err(x) = writer.send(res).await {
                error!("Failed to send response through unix connection: {}", x);
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::InmemoryStream;
    use std::{pin::Pin, time::Duration};

    fn make_session() -> (Transport<InmemoryStream>, Session<InmemoryStream>) {
        let (t1, t2) = Transport::make_pair();
        (t1, Session::initialize(t2).unwrap())
    }

    fn make_transport_stream() -> (
        mpsc::Sender<Transport<InmemoryStream>>,
        Pin<Box<dyn Stream<Item = Transport<InmemoryStream>> + Send>>,
    ) {
        let (tx, rx) = mpsc::channel::<Transport<InmemoryStream>>(1);
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
        let (mut t1, t2) = Transport::make_pair();
        tx.send(t2).await.unwrap();

        // Send a request
        let req = Request::new("test-tenant", vec![RequestData::SystemInfo {}]);
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
        let (mut t1, t2) = Transport::make_pair();
        tx.send(t2).await.unwrap();

        // Send over a second "connection"
        let (mut t2, t3) = Transport::make_pair();
        tx.send(t3).await.unwrap();

        // Send a request to mark the tenant of the first connection
        t1.send(Request::new(
            "test-tenant-1",
            vec![RequestData::SystemInfo {}],
        ))
        .await
        .unwrap();

        // Send a request to mark the tenant of the second connection
        t2.send(Request::new(
            "test-tenant-2",
            vec![RequestData::SystemInfo {}],
        ))
        .await
        .unwrap();

        // Clear out the transport channel (outbound of session)
        // NOTE: Because our test stream uses a buffer size of 1, we have to clear out the
        //       outbound data from the earlier requests before we can send back a response
        let _ = transport.receive::<Request>().await.unwrap().unwrap();
        let _ = transport.receive::<Request>().await.unwrap().unwrap();

        // Send a response back to a singular connection based on the tenant
        let res = Response::new("test-tenant-2", None, vec![ResponseData::Ok]);
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
