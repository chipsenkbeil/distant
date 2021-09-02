use crate::{
    client::Session,
    constants::{CLIENT_BROADCAST_CHANNEL_CAPACITY, CONN_HANDSHAKE_TIMEOUT_MILLIS},
    data::{Request, RequestData, Response, ResponseData},
    net::{DataStream, Listener, ListenerCtx, Transport, TransportReadHalf, TransportWriteHalf},
    server::utils::{ConnTracker, ShutdownTask},
};
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
    pub fn initialize<T1, T2, L>(
        mut session: Session<T1>,
        listener: L,
        shutdown_after: Option<Duration>,
    ) -> io::Result<Self>
    where
        T1: DataStream + 'static,
        T2: DataStream + Send + 'static,
        L: Listener<Conn = T2> + 'static,
    {
        let conns: Arc<Mutex<HashMap<usize, Conn>>> = Arc::new(Mutex::new(HashMap::new()));

        // Spawn task to send server responses to the appropriate connections
        let conns_2 = Arc::clone(&conns);
        debug!("Spawning response broadcast task");
        let mut broadcast = session.broadcast.take().unwrap();
        let broadcast_task = tokio::spawn(async move {
            while let Some(res) = broadcast.recv().await {
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
        let forward_task = tokio::spawn(async move {
            while let Some(req) = req_rx.recv().await {
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
                let ctx = ListenerCtx {
                    auth_key: None,
                    timeout: Duration::from_millis(CONN_HANDSHAKE_TIMEOUT_MILLIS),
                };
                loop {
                    match listener.accept(&ctx).await {
                        Ok(transport_f) => tokio::spawn(async move {
                            match transport_f.await {
                                Ok(transport) => {
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
                                Err(x) => {
                                    error!("Connection handshake timed out: {}", x);
                                }
                            }
                        }),
                        Err(x) => {
                            debug!("Listener has closed: {}", x);
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
    use crate::net::{InmemoryStream, SecretKey};
    use serde::{de::DeserializeOwned, Serialize};
    use std::{marker::PhantomData, time::Duration};

    async fn timeout<F: std::future::Future<Output = T>, T>(f: F) -> T {
        tokio::select! {
            res = f => {
                res
            }
            _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => {
                panic!("Time elapsed waiting on future");
            }
        }
    }

    /// Sends data to stream as if it is arriving from the outside
    struct Incoming<T: Serialize>(mpsc::Sender<Vec<u8>>, PhantomData<T>);
    impl<T: Serialize> Incoming<T> {
        pub fn new(tx: mpsc::Sender<Vec<u8>>) -> Self {
            Self(tx, PhantomData)
        }

        pub async fn send(&self, data: T) {
            self.0
                .send(serde_cbor::to_vec(&data).expect("Failed to encode data"))
                .await
                .expect("Failed to send data")
        }
    }

    /// Receives data from the stream as if it is being sent to the outside
    struct Outgoing<T: DeserializeOwned>(mpsc::Receiver<Vec<u8>>, PhantomData<T>);
    impl<T: DeserializeOwned> Outgoing<T> {
        pub fn new(rx: mpsc::Receiver<Vec<u8>>) -> Self {
            Self(rx, PhantomData)
        }

        pub async fn recv(&mut self) -> Option<T> {
            self.0
                .recv()
                .await
                .map(|data| serde_cbor::from_slice(&data).expect("Failed to decode data"))
        }
    }

    fn make_client_stream() -> (Incoming<Response>, Outgoing<Request>, InmemoryStream) {
        let (tx, rx, stream) = InmemoryStream::make(1);
        (Incoming::new(tx), Outgoing::new(rx), stream)
    }

    fn make_server_stream() -> (Incoming<Request>, Outgoing<Response>, InmemoryStream) {
        let (tx, rx, stream) = InmemoryStream::make(1);
        (Incoming::new(tx), Outgoing::new(rx), stream)
    }

    fn make_client_transport() -> (
        Incoming<Response>,
        Outgoing<Request>,
        Transport<InmemoryStream>,
    ) {
        let (tx, rx, stream) = make_client_stream();
        let transport = Transport::new(stream, None, Arc::new(SecretKey::default()));
        (tx, rx, transport)
    }

    #[tokio::test]
    async fn wait_should_return_ok_when_all_inner_tasks_complete() {
        let (res_tx, req_rx, transport) = make_client_transport();
        let session = Session::initialize(transport).unwrap();

        let (tx, rx) = mpsc::channel::<InmemoryStream>(1);
        let listener = Mutex::new(rx);

        let server = RelayServer::initialize(session, listener, None).unwrap();

        // Conclude all server tasks by closing out the listener & session
        drop(res_tx);
        drop(req_rx);
        drop(tx);

        let result = server.wait().await;
        assert!(result.is_ok(), "Unexpected result: {:?}", result);
    }

    #[tokio::test]
    async fn wait_should_return_error_when_server_aborted() {
        let (_res_tx, _req_rx, transport) = make_client_transport();
        let session = Session::initialize(transport).unwrap();

        let (_tx, rx) = mpsc::channel::<InmemoryStream>(1);
        let listener = Mutex::new(rx);

        let server = RelayServer::initialize(session, listener, None).unwrap();
        server.abort().await;

        match server.wait().await {
            Err(x) if x.is_cancelled() => {}
            x => panic!("Unexpected result: {:?}", x),
        }
    }

    #[tokio::test]
    async fn server_should_shutdown_if_no_connections_after_shutdown_duration() {
        let (_res_tx, _req_rx, transport) = make_client_transport();
        let session = Session::initialize(transport).unwrap();

        let (_tx, rx) = mpsc::channel::<InmemoryStream>(1);
        let listener = Mutex::new(rx);

        let server =
            RelayServer::initialize(session, listener, Some(Duration::from_millis(50))).unwrap();

        // TODO: Hanging due to broadcast and/or forward tasks not completed
        let result = timeout(server.wait()).await;
        assert!(result.is_ok(), "Unexpected result: {:?}", result);
    }
}
