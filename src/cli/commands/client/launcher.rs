use super::Format;
use crate::{
    cli::{
        client::{MsgReceiver, MsgSender},
        CliResult,
    },
    config::{BindAddress, ClientLaunchConfig},
    constants::USERNAME,
};
use distant_core::{
    net::{AuthErrorKind, AuthQuestion, AuthRequest, AuthResponse, AuthVerifyKind},
    Destination, DistantSingleKeyCredentials,
};
use log::*;
use std::{collections::HashMap, io};
use tokio::process::Command;

pub struct Launcher;

impl Launcher {
    pub async fn spawn_remote_server(
        format: Format,
        config: ClientLaunchConfig,
        destination: impl AsRef<Destination>,
    ) -> CliResult<DistantSingleKeyCredentials> {
        #[cfg(any(feature = "libssh", feature = "ssh2"))]
        if config.ssh.external {
            external_spawn_remote_server(config, destination).await
        } else {
            native_spawn_remote_server(format, config, destination).await
        }

        #[cfg(not(any(feature = "libssh", feature = "ssh2")))]
        external_spawn_remote_server(config, destination).await
    }
}

/// Spawns a remote server using native ssh library that listens for requests
///
/// Returns the session associated with the server
#[cfg(any(feature = "libssh", feature = "ssh2"))]
async fn native_spawn_remote_server(
    format: Format,
    config: ClientLaunchConfig,
    destination: impl AsRef<Destination>,
) -> CliResult<DistantSingleKeyCredentials> {
    let destination = destination.as_ref();
    trace!(
        "native_spawn_remote_server({:?}, {:?}, {})",
        format,
        config,
        destination
    );
    use distant_ssh2::{DistantLaunchOpts, Ssh, SshAuthHandler, SshOpts};

    let host = destination
        .host()
        .map(ToString::to_string)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Missing host"))?;

    // Build our options based on cli input
    let mut opts = SshOpts::default();
    if let Some(path) = config.ssh.identity_file {
        opts.identity_files.push(path);
    }

    opts.port = destination.port().or(config.ssh.port);
    opts.user = Some(
        destination
            .username()
            .map(ToString::to_string)
            .or(config.ssh.username)
            .unwrap_or_else(|| USERNAME.to_string()),
    );

    debug!("Connecting to {} {:#?}", host, opts);
    let mut ssh = Ssh::connect(host.as_str(), opts)?;

    debug!("Authenticating against {}", host);
    ssh.authenticate(match format {
        Format::Shell => SshAuthHandler::default(),
        Format::Json => {
            let tx = MsgSender::from_stdout();
            let tx_2 = tx.clone();
            let tx_3 = tx.clone();
            let tx_4 = tx.clone();
            let rx = MsgReceiver::from_stdin();
            let rx_2 = rx.clone();

            SshAuthHandler {
                on_authenticate: Box::new(move |ev| {
                    let mut extra = HashMap::new();
                    extra.insert("instructions".to_string(), ev.instructions);
                    extra.insert("username".to_string(), ev.username);

                    let mut questions = Vec::new();
                    for prompt in ev.prompts {
                        let mut extra = HashMap::new();
                        extra.insert("echo".to_string(), prompt.echo.to_string());

                        questions.push(AuthQuestion {
                            text: prompt.prompt,
                            extra,
                        });
                    }

                    let _ = tx.send_blocking(&AuthRequest::Challenge { questions, extra });

                    let msg: AuthResponse = rx.recv_blocking()?;
                    match msg {
                        AuthResponse::Challenge { answers } => Ok(answers),
                        x => {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidInput,
                                format!("Invalid response received: {:?}", x),
                            ))
                        }
                    }
                }),
                on_banner: Box::new(move |banner| {
                    let _ = tx_2.send_blocking(&AuthRequest::Info {
                        text: banner.to_string(),
                    });
                }),
                on_host_verify: Box::new(move |host| {
                    let _ = tx_3.send_blocking(&AuthRequest::Verify {
                        kind: AuthVerifyKind::Host,
                        text: host.to_string(),
                    })?;

                    let msg: AuthResponse = rx_2.recv_blocking()?;
                    match msg {
                        AuthResponse::Verify { valid } => Ok(valid),
                        x => {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidInput,
                                format!("Invalid response received: {:?}", x),
                            ))
                        }
                    }
                }),
                on_error: Box::new(move |err| {
                    let _ = tx_4.send_blocking(&AuthRequest::Error {
                        kind: AuthErrorKind::Unknown,
                        text: err.to_string(),
                    });
                }),
            }
        }
    })
    .await?;

    debug!("Launching for {}", host);
    let credentials = ssh
        .launch(DistantLaunchOpts {
            binary: config.distant.bin.unwrap_or_else(|| "distant".to_string()),
            args: config.distant.args.unwrap_or_default(),
            ..Default::default()
        })
        .await?;

    Ok(credentials)
}

/// Spawns a remote server using external ssh command that listens for requests
///
/// Returns the session associated with the server
async fn external_spawn_remote_server(
    config: ClientLaunchConfig,
    destination: impl AsRef<Destination>,
) -> CliResult<DistantSingleKeyCredentials> {
    let destination = destination.as_ref();
    trace!(
        "external_spawn_remote_server({:?}, {})",
        config,
        destination
    );
    let distant_command = format!(
        "{} server listen --daemon --host {} {}",
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
            String::from_utf8(out.stderr)
                .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?
                .trim()
                .to_string(),
        )
        .into());
    }

    // Parse our output for the specific credentials line
    // NOTE: The host provided on this line isn't valid, so we fill it in with our actual host
    let out = String::from_utf8(out.stdout)
        .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?
        .trim()
        .to_string();
    let credentials = out
        .lines()
        .find_map(|line| line.parse::<DistantSingleKeyCredentials>().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Missing launch info"))?;
    Ok(credentials)
}
