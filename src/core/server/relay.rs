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
                loop {
                    match listener.accept().await {
                        Ok(stream) => {
                            let result = Conn::initialize(
                                stream,
                                req_tx.clone(),
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

    pub async fn wait(self) -> Result<(), JoinError> {
        match tokio::try_join!(self.accept_task, self.broadcast_task, self.forward_task) {
            Ok(_) => Ok(()),
            Err(x) => Err(x),
        }
    }

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
        stream: T,
        req_tx: mpsc::Sender<Request>,
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
        let (res_tx, res_rx) = mpsc::channel::<Response>(CLIENT_BROADCAST_CHANNEL_CAPACITY);
        let state_2 = Arc::clone(&state);
        let res_task = tokio::spawn(async move {
            handle_conn_outgoing(id, state_2, t_write, tenant_rx, res_rx).await;
        });

        // Spawn task to continually read requests from connection and forward
        // them along to be sent via the session
        let req_tx = req_tx.clone();
        let state_2 = Arc::clone(&state);
        let req_task = tokio::spawn(async move {
            if let Some(ct) = ct.as_ref() {
                ct.lock().await.increment();
            }
            handle_conn_incoming(id, state_2, t_read, tenant_tx, req_tx).await;
            if let Some(ct) = ct.as_ref() {
                ct.lock().await.decrement();
            }
            debug!("<Conn @ {}> Disconnected", id);
        });

        Ok(Self {
            id,
            req_task,
            res_task,
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

    #[test]
    fn wait_should_return_ok_when_all_inner_tasks_complete() {
        todo!();
    }

    #[test]
    fn wait_should_return_error_when_server_aborted() {
        todo!();
    }

    #[test]
    fn abort_should_abort_inner_tasks_and_all_connections() {
        todo!();
    }

    #[test]
    fn server_should_shutdown_if_no_connections_after_shutdown_duration() {
        todo!();
    }

    #[test]
    fn server_shutdown_should_abort_all_connections() {
        todo!();
    }

    #[test]
    fn server_should_forward_connection_requests_to_session() {
        todo!();
    }

    #[test]
    fn server_should_forward_session_responses_to_connection_with_matching_tenant() {
        todo!();
    }

    #[test]
    fn connection_abort_should_abort_inner_tasks() {
        todo!();
    }

    #[test]
    fn connection_abort_should_send_process_kill_requests_through_session() {
        todo!();
    }
}
