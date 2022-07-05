use super::Format;
use crate::{
    cli::{CliError, CliResult},
    config::{BindAddress, ClientLaunchConfig},
    constants::USERNAME,
};
use distant_core::Destination;
use log::*;
use std::io;
use tokio::process::Command;

async fn spawn_remote_server(
    config: ClientLaunchConfig,
    destination: Destination,
) -> CliResult<SessionInfo> {
    #[cfg(any(feature = "libssh", feature = "ssh2"))]
    if config.ssh.external {
        external_spawn_remote_server(config, destination).await
    } else {
        native_spawn_remote_server(config, destination).await
    }

    #[cfg(not(any(feature = "libssh", feature = "ssh2")))]
    external_spawn_remote_server(config, destination).await
}

/// Spawns a remote server using native ssh library that listens for requests
///
/// Returns the session associated with the server
#[cfg(any(feature = "libssh", feature = "ssh2"))]
async fn native_spawn_remote_server(
    config: ClientLaunchConfig,
    destination: Destination,
) -> CliResult<SessionInfo> {
    trace!("native_spawn_remote_server({:?})", cmd);
    use distant_ssh2::{
        IntoDistantSessionOpts, SshAuthEvent, SshAuthHandler, SshSession, SshSessionOpts,
    };

    let host = cmd.host;

    // Build our options based on cli input
    let mut opts = Ssh2SessionOpts::default();
    if let Some(path) = cmd.identity_file {
        opts.identity_files.push(path);
    }
    opts.port = Some(cmd.port);
    opts.user = Some(cmd.username);

    debug!("Connecting to {} {:#?}", host, opts);
    let mut ssh_session = Ssh2Session::connect(host.as_str(), opts)?;

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
    config: ClientLaunchConfig,
    destination: Destination,
) -> CliResult<SessionInfo> {
    let distant_command = format!(
        "{} listen --host {} {}",
        config.distant.bin.unwrap_or_else(|| "distant".to_string()),
        config.distant.bind_server.unwrap_or(BindAddress::Ssh),
        config.distant.args.unwrap_or_default(),
    );
    let ssh_command = format!(
        "{} -o StrictHostKeyChecking=no ssh://{}@{}:{} {} '{}'",
        config.ssh.bin.unwrap_or_else(|| "ssh".to_string()),
        destination
            .username()
            .map(ToString::to_string)
            .or(config.ssh.username)
            .unwrap_or_else(|| USERNAME.to_string()),
        destination
            .host()
            .map(ToString::to_string)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Missing host"))?,
        destination.port().or(config.ssh.port).unwrap_or(22),
        config
            .ssh
            .identity_file
            .map(|f| format!("-i {}", f.as_path().display()))
            .unwrap_or_default(),
        if config.distant.no_shell {
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
        return Err(io::Error::new(
            io::ErrorKind::Other,
            String::from_utf8(out.stderr)?.trim().to_string(),
        ))
        .into();
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
