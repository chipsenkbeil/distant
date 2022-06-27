use crate::{
    environment,
    exit::{ExitCode, ExitCodeError},
    msg::{MsgReceiver, MsgSender},
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
    let is_daemon = !cmd.foreground;

    let session_file = cmd.session_data.session_file.clone();
    let session_socket = cmd.session_data.session_socket.clone();
    let fail_if_socket_exists = cmd.fail_if_socket_exists;
    let timeout = opt.to_timeout_duration();
    let shutdown_after = cmd.to_shutdown_after_duration();

    let session = rt.block_on(async { spawn_remote_server(cmd, opt).await })?;

    // Handle sharing resulting session in different ways
    match session_output {
        SessionOutput::Environment => {
            debug!("Outputting session to environment");
            environment::print_environment(&session)
        }
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

#[cfg(unix)]
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
        TransportListener::new(listener, |stream| Transport::new(stream, PlainCodec::new()))
            .into_stream();

    let server = RelayServer::initialize(session, Box::pin(stream), shutdown_after)?;
    server
        .wait()
        .await
        .map_err(|x| io::Error::new(io::ErrorKind::Other, x))
}

async fn spawn_remote_server(cmd: LaunchSubcommand, opt: CommonOpt) -> Result<SessionInfo, Error> {
    #[cfg(any(feature = "libssh", feature = "ssh2"))]
    if cmd.external_ssh {
        external_spawn_remote_server(cmd, opt).await
    } else {
        native_spawn_remote_server(cmd, opt).await
    }

    #[cfg(not(any(feature = "libssh", feature = "ssh2")))]
    external_spawn_remote_server(cmd, opt).await
}

/// Spawns a remote server using native ssh library that listens for requests
///
/// Returns the session associated with the server
#[cfg(any(feature = "libssh", feature = "ssh2"))]
async fn native_spawn_remote_server(
    cmd: LaunchSubcommand,
    _opt: CommonOpt,
) -> Result<SessionInfo, Error> {
    trace!("native_spawn_remote_server({:?})", cmd);
    use distant_ssh2::{
        IntoDistantSessionOpts, Ssh2AuthEvent, Ssh2AuthHandler, Ssh2Session, Ssh2SessionOpts,
    };

    let host = cmd.host;

    // Build our options based on cli input
    let mut opts = Ssh2SessionOpts::default();
    if let Some(path) = cmd.identity_file {
        opts.identity_files.push(path);
    }
    opts.backend = cmd.ssh_backend;
    opts.port = Some(cmd.port);
    opts.user = Some(cmd.username);

    debug!("Connecting to {} {:#?}", host, opts);
    let mut ssh_session = Ssh2Session::connect(host.as_str(), opts)?;

    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    #[serde(tag = "type")]
    enum SshMsg {
        #[serde(rename = "ssh_authenticate")]
        Authenticate(Ssh2AuthEvent),
        #[serde(rename = "ssh_authenticate_answer")]
        AuthenticateAnswer { answers: Vec<String> },
        #[serde(rename = "ssh_banner")]
        Banner { text: String },
        #[serde(rename = "ssh_host_verify")]
        HostVerify { host: String },
        #[serde(rename = "ssh_host_verify_answer")]
        HostVerifyAnswer { answer: bool },
        #[serde(rename = "ssh_error")]
        Error { msg: String },
    }

    debug!("Authenticating against {}", host);
    ssh_session
        .authenticate(match cmd.format {
            Format::Shell => Ssh2AuthHandler::default(),
            Format::Json => {
                let tx = MsgSender::from_stdout();
                let tx_2 = tx.clone();
                let tx_3 = tx.clone();
                let tx_4 = tx.clone();
                let rx = MsgReceiver::from_stdin();
                let rx_2 = rx.clone();

                Ssh2AuthHandler {
                    on_authenticate: Box::new(move |ev| {
                        let _ = tx.send_blocking(&SshMsg::Authenticate(ev));

                        let msg: SshMsg = rx.recv_blocking()?;
                        match msg {
                            SshMsg::AuthenticateAnswer { answers } => Ok(answers),
                            x => {
                                return Err(io::Error::new(
                                    io::ErrorKind::InvalidInput,
                                    format!("Invalid response received: {:?}", x),
                                ))
                            }
                        }
                    }),
                    on_banner: Box::new(move |banner| {
                        let _ = tx_2.send_blocking(&SshMsg::Banner {
                            text: banner.to_string(),
                        });
                    }),
                    on_host_verify: Box::new(move |host| {
                        let _ = tx_3.send_blocking(&SshMsg::HostVerify {
                            host: host.to_string(),
                        })?;

                        let msg: SshMsg = rx_2.recv_blocking()?;
                        match msg {
                            SshMsg::HostVerifyAnswer { answer } => Ok(answer),
                            x => {
                                return Err(io::Error::new(
                                    io::ErrorKind::InvalidInput,
                                    format!("Invalid response received: {:?}", x),
                                ))
                            }
                        }
                    }),
                    on_error: Box::new(move |err| {
                        let _ = tx_4.send_blocking(&SshMsg::Error {
                            msg: err.to_string(),
                        });
                    }),
                }
            }
        })
        .await?;

    debug!("Mapping session for {}", host);
    let session_info = ssh_session
        .into_distant_session_info(IntoDistantSessionOpts {
            binary: cmd.distant,
            args: cmd.extra_server_args.unwrap_or_default(),
            ..Default::default()
        })
        .await?;

    Ok(session_info)
}

/// Spawns a remote server using external ssh command that listens for requests
///
/// Returns the session associated with the server
async fn external_spawn_remote_server(
    cmd: LaunchSubcommand,
    _opt: CommonOpt,
) -> Result<SessionInfo, Error> {
    let distant_command = format!(
        "{} listen --host {} {}",
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
