use crate::{
    cli::opt::{CommonOpt, LaunchSubcommand, Mode, SessionOutput},
    core::{
        constants::CLIENT_BROADCAST_CHANNEL_CAPACITY,
        data::{Request, Response},
        net::{Client, Transport, TransportReadHalf, TransportWriteHalf},
        session::{Session, SessionFile},
        utils,
    },
};
use derive_more::{Display, Error, From};
use fork::{daemon, Fork};
use hex::FromHexError;
use log::*;
use orion::errors::UnknownCryptoError;
use std::{marker::Unpin, path::Path, string::FromUtf8Error};
use tokio::{
    io::{self, AsyncRead, AsyncWrite},
    process::Command,
    sync::{broadcast, mpsc, oneshot},
};

#[derive(Debug, Display, Error, From)]
pub enum Error {
    #[display(fmt = "Missing data for session")]
    MissingSessionData,

    ForkError(#[error(not(source))] i32),
    BadKey(UnknownCryptoError),
    HexError(FromHexError),
    IoError(io::Error),
    Utf8Error(FromUtf8Error),
}

pub fn run(cmd: LaunchSubcommand, opt: CommonOpt) -> Result<(), Error> {
    let rt = tokio::runtime::Runtime::new()?;
    let session_output = cmd.session;
    let mode = cmd.mode;
    let is_daemon = cmd.daemon;

    let session_file = cmd.session_data.session_file.clone();
    let session_socket = cmd.session_data.session_socket.clone();

    let session = rt.block_on(async { spawn_remote_server(cmd, opt).await })?;

    // Handle sharing resulting session in different ways
    match session_output {
        SessionOutput::File => {
            debug!("Outputting session to {:?}", session_file);
            rt.block_on(async { SessionFile::new(session_file, session).save().await })?
        }
        SessionOutput::Keep => {
            debug!("Entering interactive loop over stdin");
            rt.block_on(async { keep_loop(session, mode).await })?
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
                    let rt = tokio::runtime::Runtime::new()?;
                    rt.block_on(async { socket_loop(session_socket, session).await })?
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
            rt.block_on(async { socket_loop(session_socket, session).await })?
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

async fn keep_loop(session: Session, mode: Mode) -> io::Result<()> {
    use crate::cli::subcommand::action::inner;
    match Client::tcp_connect(session).await {
        Ok(client) => {
            let config = match mode {
                Mode::Json => inner::LoopConfig::Json,
                Mode::Shell => inner::LoopConfig::Shell,
            };
            inner::interactive_loop(client, utils::new_tenant(), config).await
        }
        Err(x) => Err(x),
    }
}

#[cfg(unix)]
async fn socket_loop(socket_path: impl AsRef<Path>, session: Session) -> io::Result<()> {
    // We need to form a connection with the actual server to forward requests
    // and responses between connections
    debug!("Connecting to {} {}", session.host, session.port);
    let mut client = Client::tcp_connect(session).await?;

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
                "Forwarding request of type {} to server",
                req.payload.as_ref()
            );
            if let Err(x) = client.fire(req).await {
                error!("Client failed to send request: {:?}", x);
                break;
            }
        }
    });

    // Continue to receive connections over the unix socket, store them in our
    // connection mapping
    debug!("Binding to unix socket: {:?}", socket_path.as_ref());
    let listener = tokio::net::UnixListener::bind(socket_path)?;

    while let Ok((conn, addr)) = listener.accept().await {
        // Establish a proper connection via a handshake, discarding the connection otherwise
        let transport = match Transport::from_handshake(conn, None).await {
            Ok(transport) => transport,
            Err(x) => {
                error!("<Client @ {:?}> Failed handshake: {}", addr, x);
                continue;
            }
        };
        let (t_read, t_write) = transport.into_split();

        // Used to alert our response task of the connection's tenant name
        // based on the first
        let (tenant_tx, tenant_rx) = oneshot::channel();

        // Spawn task to continually receive responses from the client that
        // may or may not be relevant to the connection, which will filter
        // by tenant and then along any response that matches
        let res_rx = broadcaster.subscribe();
        tokio::spawn(async move {
            handle_conn_outgoing(addr, t_write, tenant_rx, res_rx).await;
        });

        // Spawn task to continually read requests from connection and forward
        // them along to be sent via the client
        let req_tx = req_tx.clone();
        tokio::spawn(async move {
            handle_conn_incoming(t_read, tenant_tx, req_tx).await;
        });
    }

    Ok(())
}

/// Conn::Request -> Client::Fire
async fn handle_conn_incoming<T>(
    mut reader: TransportReadHalf<T>,
    tenant_tx: oneshot::Sender<String>,
    req_tx: mpsc::Sender<Request>,
) where
    T: AsyncRead + Unpin,
{
    macro_rules! process_req {
        ($on_success:expr) => {
            match reader.receive::<Request>().await {
                Ok(Some(req)) => {
                    $on_success(&req);
                    if let Err(x) = req_tx.send(req).await {
                        error!(
                            "Failed to pass along request received on unix socket: {:?}",
                            x
                        );
                        return;
                    }
                }
                Ok(None) => return,
                Err(x) => {
                    error!("Failed to receive request from unix stream: {:?}", x);
                    return;
                }
            }
        };
    }

    // NOTE: Have to acquire our first request outside our loop since the oneshot
    //       sender of the tenant's name is consuming
    process_req!(|req: &Request| {
        if let Err(x) = tenant_tx.send(req.tenant.clone()) {
            error!("Failed to send along acquired tenant name: {:?}", x);
            return;
        }
    });

    loop {
        process_req!(|_| {});
    }
}

async fn handle_conn_outgoing<T>(
    addr: tokio::net::unix::SocketAddr,
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
        debug!("Associated tenant {} with conn {:?}", tenant, addr);
        loop {
            match res_rx.recv().await {
                // Forward along responses that are for our connection
                Ok(res) if res.tenant == tenant => {
                    debug!(
                        "Conn {:?} being sent response of type {}",
                        addr,
                        res.payload.as_ref()
                    );
                    if let Err(x) = writer.send(res).await {
                        error!("Failed to send response through unix connection: {}", x);
                        break;
                    }
                }
                // Skip responses that are not for our connection
                Ok(_) => {}
                Err(x) => {
                    error!(
                        "Conn {:?} failed to receive broadcast response: {}",
                        addr, x
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
        "{} -o StrictHostKeyChecking=no ssh://{}@{}:{} {} {}",
        cmd.ssh,
        cmd.username,
        cmd.host.as_str(),
        cmd.port,
        cmd.identity_file
            .map(|f| format!("-i {}", f.as_path().display()))
            .unwrap_or_default(),
        distant_command.trim(),
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
