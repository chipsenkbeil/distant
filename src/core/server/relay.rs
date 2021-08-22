use crate::core::{
    client::Session,
    constants::CLIENT_BROADCAST_CHANNEL_CAPACITY,
    data::{Request, RequestData, Response, ResponseData},
    net::{DataStream, Listener, Transport, TransportReadHalf, TransportWriteHalf},
    server::utils::{ConnTracker, ShutdownTask},
};
use log::*;
use std::{collections::HashMap, marker::Unpin, sync::Arc};
use tokio::{
    io::{self, AsyncRead, AsyncWrite},
    sync::{broadcast, mpsc, oneshot, Mutex},
    task::{JoinError, JoinHandle},
    time::Duration,
};

/// Represents a server that relays requests & responses between connections and the
/// actual server
pub struct RelayServer {
    forward_task: JoinHandle<()>,
    accept_task: JoinHandle<()>,
    conns: Arc<Mutex<HashMap<usize, Conn>>>,
}

impl RelayServer {
    pub async fn initialize<T1, T2, L>(
        mut session: Session<T1>,
        listener: L,
        shutdown_after: Option<Duration>,
    ) -> io::Result<Self>
    where
        T1: DataStream + 'static,
        T2: DataStream + Send + 'static,
        L: Listener<Conn = T2> + 'static,
    {
        // Get a copy of our session's broadcaster so we can have each connection
        // subscribe to it for new messages filtered by tenant
        debug!("Acquiring session broadcaster");
        let broadcaster = session.to_response_broadcaster();

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
        let conns = Arc::new(Mutex::new(HashMap::new()));
        let conns_2 = Arc::clone(&conns);
        let accept_task = tokio::spawn(async move {
            let inner = async move {
                loop {
                    match listener.accept().await {
                        Ok(stream) => {
                            let result = Conn::initialize(
                                stream,
                                req_tx.clone(),
                                broadcaster.clone(),
                                tracker.as_ref().map(Arc::clone),
                            )
                            .await;

                            match result {
                                Ok(conn) => conns_2.lock().await.insert(conn.id(), conn),
                                Err(x) => {
                                    error!("Failed to initialize connection: {}", x);
                                    continue;
                                }
                            };
                        }
                        Err(x) => {
                            debug!("Listener has closed: {}", x);
                            break;
                        }
                    }
                }
            };

            tokio::select! {
                _ = inner => {}
                _ = shutdown => {
                    warn!("Reached shutdown timeout, so terminating");
                }
            }
        });

        Ok(Self {
            forward_task,
            accept_task,
            conns,
        })
    }

    pub async fn wait(self) -> Result<(), JoinError> {
        match tokio::try_join!(self.forward_task, self.accept_task) {
            Ok(_) => Ok(()),
            Err(x) => Err(x),
        }
    }

    pub async fn abort(&self) {
        self.forward_task.abort();
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
    req_task: JoinHandle<()>,
    res_task: JoinHandle<()>,
}

/// Represents state associated with a connection
#[derive(Default)]
struct ConnState {
    processes: Vec<usize>,
}

impl Conn {
    pub async fn initialize<T>(
        stream: T,
        req_tx: mpsc::Sender<Request>,
        res_broadcaster: broadcast::Sender<Response>,
        ct: Option<Arc<Mutex<ConnTracker>>>,
    ) -> io::Result<Self>
    where
        T: DataStream + 'static,
    {
        // Create a unique id to associate with the connection since its address
        // is not guaranteed to have an identifiable string
        let id: usize = rand::random();

        // Establish a proper connection via a handshake, discarding the connection otherwise
        let transport = Transport::from_handshake(stream, None).await.map_err(|x| {
            error!("<Conn @ {}> Failed handshake: {}", id, x);
            io::Error::new(io::ErrorKind::Other, x)
        })?;
        let (t_read, t_write) = transport.into_split();

        // Used to alert our response task of the connection's tenant name
        // based on the first
        let (tenant_tx, tenant_rx) = oneshot::channel();

        // Create a state we use to keep track of connection-specific data
        debug!("<Conn @ {}> Initializing internal state", id);
        let state = Arc::new(Mutex::new(ConnState::default()));

        // Spawn task to continually receive responses from the session that
        // may or may not be relevant to the connection, which will filter
        // by tenant and then along any response that matches
        let res_rx = res_broadcaster.subscribe();
        let state_2 = Arc::clone(&state);
        let res_task = tokio::spawn(async move {
            handle_conn_outgoing(id, state_2, t_write, tenant_rx, res_rx).await;
        });

        // Spawn task to continually read requests from connection and forward
        // them along to be sent via the session
        let req_tx = req_tx.clone();
        let req_task = tokio::spawn(async move {
            if let Some(ct) = ct.as_ref() {
                ct.lock().await.increment();
            }
            handle_conn_incoming(id, state, t_read, tenant_tx, req_tx).await;
            if let Some(ct) = ct.as_ref() {
                ct.lock().await.decrement();
            }
            debug!("<Conn @ {}> Disconnected", id);
        });

        Ok(Self {
            id,
            req_task,
            res_task,
        })
    }

    /// Id associated with the connection
    pub fn id(&self) -> usize {
        self.id
    }

    /// Aborts the connection from the server side
    pub fn abort(&self) {
        self.req_task.abort();
        self.res_task.abort();
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
    mut res_rx: broadcast::Receiver<Response>,
) where
    T: AsyncWrite + Unpin,
{
    // We wait for the tenant to be identified by the first request
    // before processing responses to be sent back; this is easier
    // to implement and yields the same result as we would be dropping
    // all responses before we know the tenant
    if let Ok(tenant) = tenant_rx.await {
        debug!("Associated tenant {} with conn {}", tenant, conn_id);
        loop {
            match res_rx.recv().await {
                // Forward along responses that are for our connection
                Ok(res) if res.tenant == tenant => {
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
                // Skip responses that are not for our connection
                Ok(_) => {}
                Err(x) => {
                    error!(
                        "Conn {} failed to receive broadcast response: {}",
                        conn_id, x
                    );
                    break;
                }
            }
        }
    }
}
