use crate::{
    exit::{ExitCode, ExitCodeError},
    opt::{CommonOpt, Format, LaunchSubcommand, SessionOutput},
    session::CliSession,
    utils,
};
use derive_more::{Display, Error, From};
use distant_core::{
    PlainCodec, RelayServer, Session, SessionInfo, SessionInfoFile, Transport, TransportListener,
    XChaCha20Poly1305Codec,
};
use log::*;
use std::{path::Path, string::FromUtf8Error};
use tokio::{io, process::Command, runtime::Runtime, time::Duration};

#[derive(Debug, Display, Error, From)]
pub enum Error {
    #[display(fmt = "Missing data for session")]
    MissingSessionData,

    Fork(#[error(not(source))] i32),
    Io(io::Error),
    Utf8(FromUtf8Error),
}

impl ExitCodeError for Error {
    fn to_exit_code(&self) -> ExitCode {
        match self {
            Self::MissingSessionData => ExitCode::NoInput,
            Self::Fork(_) => ExitCode::OsErr,
            Self::Io(x) => x.to_exit_code(),
            Self::Utf8(_) => ExitCode::DataErr,
        }
    }
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
            rt.block_on(async { SessionInfoFile::new(session_file, session).save().await })?
        }
        SessionOutput::Keep => {
            debug!("Entering interactive loop over stdin");
            rt.block_on(async { keep_loop(session, format, timeout).await })?
        }
        SessionOutput::Pipe => {
            debug!("Piping session to stdout");
            println!("{}", session.to_unprotected_string())
        }
        #[cfg(unix)]
        SessionOutput::Socket if is_daemon => {
            debug!(
                "Forking and entering interactive loop over unix socket {:?}",
                session_socket
            );

            // Force runtime shutdown by dropping it BEFORE forking as otherwise
            // this produces a garbage process that won't die
            drop(rt);

            run_daemon_socket(
                session_socket,
                session,
                timeout,
                fail_if_socket_exists,
                shutdown_after,
            )?;
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

#[cfg(unix)]
fn run_daemon_socket(
    session_socket: impl AsRef<Path>,
    session: SessionInfo,
    timeout: Duration,
    fail_if_socket_exists: bool,
    shutdown_after: Option<Duration>,
) -> Result<(), Error> {
    use fork::{daemon, Fork};
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
        Err(x) => return Err(Error::Fork(x)),
    }

    Ok(())
}

async fn keep_loop(info: SessionInfo, format: Format, duration: Duration) -> io::Result<()> {
    let addr = info.to_socket_addr().await?;
    let codec = XChaCha20Poly1305Codec::from(info.key);
    match Session::tcp_connect_timeout(addr, codec, duration).await {
        Ok(session) => {
            let cli_session = CliSession::new_for_stdin(utils::new_tenant(), session, format);
            cli_session.wait().await
        }
        Err(x) => Err(x),
    }
}

async fn socket_loop(
    socket_path: impl AsRef<Path>,
    info: SessionInfo,
    duration: Duration,
    fail_if_socket_exists: bool,
    shutdown_after: Option<Duration>,
) -> io::Result<()> {
    // We need to form a connection with the actual server to forward requests
    // and responses between connections
    debug!("Connecting to {} {}", info.host, info.port);
    let addr = info.to_socket_addr().await?;
    let codec = XChaCha20Poly1305Codec::from(info.key);
    let session = Session::tcp_connect_timeout(addr, codec, duration).await?;

    // Remove the socket file if it already exists
    if !fail_if_socket_exists && socket_path.as_ref().exists() {
        debug!("Removing old unix socket instance");
        tokio::fs::remove_file(socket_path.as_ref()).await?;
    }

    // Continue to receive connections over the unix socket, store them in our
    // connection mapping
    debug!("Binding to unix socket: {:?}", socket_path.as_ref());
    let listener = tokio::net::UnixListener::bind(socket_path)?;

    let stream =
        TransportListener::initialize(listener, |stream| Transport::new(stream, PlainCodec::new()))
            .into_stream();

    let server = RelayServer::initialize(session, Box::pin(stream), shutdown_after)?;
    server
        .wait()
        .await
        .map_err(|x| io::Error::new(io::ErrorKind::Other, x))
}

/// Spawns a remote server that listens for requests
///
/// Returns the session associated with the server
async fn spawn_remote_server(cmd: LaunchSubcommand, _opt: CommonOpt) -> Result<SessionInfo, Error> {
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
            // TODO: Replace this with the ssh2 library shell exec once we integrate that
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
    let mut info = out
        .lines()
        .find_map(|line| line.parse::<SessionInfo>().ok())
        .ok_or(Error::MissingSessionData)?;
    info.host = cmd.host;

    Ok(info)
}
