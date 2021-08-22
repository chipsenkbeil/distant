use crate::{
    cli::{
        opt::{CommonOpt, Format, LaunchSubcommand, SessionOutput},
        ExitCode, ExitCodeError,
    },
    core::{
        constants::CLIENT_BROADCAST_CHANNEL_CAPACITY,
        data::{Request, RequestData, Response, ResponseData},
        net::{Client, Transport, TransportReadHalf, TransportWriteHalf},
        session::{Session, SessionFile},
        utils,
    },
};
use derive_more::{Display, Error, From};
use fork::{daemon, Fork};
use log::*;
use std::{marker::Unpin, path::Path, string::FromUtf8Error, sync::Arc};
use tokio::{
    io::{self, AsyncRead, AsyncWrite},
    process::Command,
    runtime::{Handle, Runtime},
    sync::{broadcast, mpsc, oneshot, Mutex},
    time::Duration,
};

#[derive(Debug, Display, Error, From)]
pub enum Error {
    #[display(fmt = "Missing data for session")]
    MissingSessionData,

    ForkError(#[error(not(source))] i32),
    IoError(io::Error),
    Utf8Error(FromUtf8Error),
}

impl ExitCodeError for Error {
    fn to_exit_code(&self) -> ExitCode {
        match self {
            Self::MissingSessionData => ExitCode::NoInput,
            Self::ForkError(_) => ExitCode::OsErr,
            Self::IoError(x) => x.to_exit_code(),
            Self::Utf8Error(_) => ExitCode::DataErr,
        }
    }
}

/// Represents state associated with a connection
#[derive(Default)]
struct ConnState {
    processes: Vec<usize>,
}

pub fn run(cmd: LaunchSubcommand, opt: CommonOpt) -> Result<(), Error> {
    let rt = Runtime::new()?;
    let session_output = cmd.session;
    let format = cmd.format;
    let is_daemon = cmd.daemon;

    let session_file = cmd.session_data.session_file.clone();
    let session_socket = cmd.session_data.session_socket.clone();
    let fail_if_socket_exists = cmd.fail_if_socket_exists;
    let timeout = opt.to_timeout_duration();
    let shutdown_after = cmd.to_shutdown_after_duration();

    let session = rt.block_on(async { spawn_remote_server(cmd, opt).await })?;

    // Handle sharing resulting session in different ways
    match session_output {
        SessionOutput::File => {
            debug!("Outputting session to {:?}", session_file);
            rt.block_on(async { SessionFile::new(session_file, session).save().await })?
        }
        SessionOutput::Keep => {
            debug!("Entering interactive loop over stdin");
            rt.block_on(async { keep_loop(session, format, timeout).await })?
        }
        SessionOutput::Pipe => {
            debug!("Piping session to stdout");
            println!("{}", session.to_unprotected_string())
        }
        SessionOutput::Socket if is_daemon => {
            debug!(
                "Forking and entering interactive loop over unix socket {:?}",
                session_socket
            );

            // Force runtime shutdown by dropping it BEFORE forking as otherwise
            // this produces a garbage process that won't die
            drop(rt);

            match daemon(false, false) {
                Ok(Fork::Child) => {
                    // NOTE: We need to create a runtime within the forked process as
                    //       tokio's runtime doesn't support being transferred from
                    //       parent to child in a fork
                    let rt = Runtime::new()?;
                    rt.block_on(async {
                        socket_loop(
                            session_socket,
                            session,
                            timeout,
                            fail_if_socket_exists,
                            shutdown_after,
                        )
                        .await
                    })?
                }
                Ok(_) => {}
                Err(x) => return Err(Error::ForkError(x)),
            }
        }
        #[cfg(unix)]
        SessionOutput::Socket => {
            debug!(
                "Entering interactive loop over unix socket {:?}",
                session_socket
            );
            rt.block_on(async {
                socket_loop(
                    session_socket,
                    session,
                    timeout,
                    fail_if_socket_exists,
                    shutdown_after,
                )
                .await
            })?
        }
        #[cfg(not(unix))]
        SessionOutput::Socket => {
            debug!(concat!(
                "Trying to enter interactive loop over unix socket, ",
                "but not on unix platform!"
            ));
            unreachable!()
        }
    }

    Ok(())
}

async fn keep_loop(session: Session, format: Format, duration: Duration) -> io::Result<()> {
    use crate::cli::subcommand::action::inner;
    match Client::tcp_connect_timeout(session, duration).await {
        Ok(client) => {
            let config = match format {
                Format::Json => inner::LoopConfig::Json,
                Format::Shell => inner::LoopConfig::Shell,
            };
            inner::interactive_loop(client, utils::new_tenant(), config).await
        }
        Err(x) => Err(x),
    }
}

#[cfg(unix)]
async fn socket_loop(
    socket_path: impl AsRef<Path>,
    session: Session,
    duration: Duration,
    fail_if_socket_exists: bool,
    shutdown_after: Option<Duration>,
) -> io::Result<()> {
    // We need to form a connection with the actual server to forward requests
    // and responses between connections
    debug!("Connecting to {} {}", session.host, session.port);
    let mut client = Client::tcp_connect_timeout(session, duration).await?;

    // Get a copy of our client's broadcaster so we can have each connection
    // subscribe to it for new messages filtered by tenant
    debug!("Acquiring client broadcaster");
    let broadcaster = client.to_response_broadcaster();

    // Spawn task to send to the server requests from connections
    debug!("Spawning request forwarding task");
    let (req_tx, mut req_rx) = mpsc::channel::<Request>(CLIENT_BROADCAST_CHANNEL_CAPACITY);
    tokio::spawn(async move {
        while let Some(req) = req_rx.recv().await {
            debug!(
                "Forwarding request of type{} {} to server",
                if req.payload.len() > 1 { "s" } else { "" },
                req.to_payload_type_string()
            );
            if let Err(x) = client.fire_timeout(req, duration).await {
                error!("Client failed to send request: {:?}", x);
                break;
            }
        }
    });

    // Remove the socket file if it already exists
    if !fail_if_socket_exists && socket_path.as_ref().exists() {
        debug!("Removing old unix socket instance");
        tokio::fs::remove_file(socket_path.as_ref()).await?;
    }

    // Continue to receive connections over the unix socket, store them in our
    // connection mapping
    debug!("Binding to unix socket: {:?}", socket_path.as_ref());
    let listener = tokio::net::UnixListener::bind(socket_path)?;

    let (ct, notify) = utils::new_shutdown_task(Handle::current(), shutdown_after);

    loop {
        tokio::select! {
            result = listener.accept() => {match result {
                Ok((conn, _)) => {
                    // Create a unique id to associate with the connection since its address
                    // is not guaranteed to have an identifiable string
                    let conn_id: usize = rand::random();

                    // Establish a proper connection via a handshake, discarding the connection otherwise
                    let transport = match Transport::from_handshake(conn, None).await {
                        Ok(transport) => transport,
                        Err(x) => {
                            error!("<Client @ {:?}> Failed handshake: {}", conn_id, x);
                            continue;
                        }
                    };
                    let (t_read, t_write) = transport.into_split();

                    // Used to alert our response task of the connection's tenant name
                    // based on the first
                    let (tenant_tx, tenant_rx) = oneshot::channel();

                    // Create a state we use to keep track of connection-specific data
                    debug!("<Client @ {}> Initializing internal state", conn_id);
                    let state = Arc::new(Mutex::new(ConnState::default()));

                    // Spawn task to continually receive responses from the client that
                    // may or may not be relevant to the connection, which will filter
                    // by tenant and then along any response that matches
                    let res_rx = broadcaster.subscribe();
                    let state_2 = Arc::clone(&state);
                    tokio::spawn(async move {
                        handle_conn_outgoing(conn_id, state_2, t_write, tenant_rx, res_rx).await;
                    });

                    // Spawn task to continually read requests from connection and forward
                    // them along to be sent via the client
                    let req_tx = req_tx.clone();
                    let ct_2 = Arc::clone(&ct);
                    tokio::spawn(async move {
                        ct_2.lock().await.increment();
                        handle_conn_incoming(conn_id, state, t_read, tenant_tx, req_tx).await;
                        ct_2.lock().await.decrement();
                        debug!("<Client @ {:?}> Disconnected", conn_id);
                    });
                }
                Err(x) => {
                    error!("Listener failed: {}", x);
                    break;
                }
            }}
            _ = notify.notified() => {
                warn!("Reached shutdown timeout, so terminating");
                break;
            }
        }
    }

    Ok(())
}

/// Conn::Request -> Client::Fire
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
        error!("<Client @ {}> Failed to send kill signals: {}", conn_id, x);
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

/// Spawns a remote server that listens for requests
///
/// Returns the session associated with the server
async fn spawn_remote_server(cmd: LaunchSubcommand, _opt: CommonOpt) -> Result<Session, Error> {
    let distant_command = format!(
        "{} listen --daemon --host {} {}",
        cmd.distant,
        cmd.bind_server,
        cmd.extra_server_args.unwrap_or_default(),
    );
    let ssh_command = format!(
        "{} -o StrictHostKeyChecking=no ssh://{}@{}:{} {} '{}'",
        cmd.ssh,
        cmd.username,
        cmd.host.as_str(),
        cmd.port,
        cmd.identity_file
            .map(|f| format!("-i {}", f.as_path().display()))
            .unwrap_or_default(),
        if cmd.no_shell {
            distant_command.trim().to_string()
        } else {
            // TODO: Do we need to try to escape single quotes here because of extra_server_args?
            format!("echo {} | $SHELL -l", distant_command.trim())
        },
    );
    let out = Command::new("sh")
        .arg("-c")
        .arg(ssh_command)
        .output()
        .await?;

    // If our attempt to run the program via ssh failed, report it
    if !out.status.success() {
        return Err(Error::from(io::Error::new(
            io::ErrorKind::Other,
            String::from_utf8(out.stderr)?.trim().to_string(),
        )));
    }

    // Parse our output for the specific session line
    // NOTE: The host provided on this line isn't valid, so we fill it in with our actual host
    let out = String::from_utf8(out.stdout)?.trim().to_string();
    let mut session = out
        .lines()
        .find_map(|line| line.parse::<Session>().ok())
        .ok_or(Error::MissingSessionData)?;
    session.host = cmd.host;

    Ok(session)
}
