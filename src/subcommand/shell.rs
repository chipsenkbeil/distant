use crate::{
    exit::{ExitCode, ExitCodeError},
    link::RemoteProcessLink,
    opt::{CommonOpt, ShellSubcommand},
    subcommand::CommandRunner,
    utils,
};
use derive_more::{Display, Error, From};
use distant_core::{LspMsg, PtySize, RemoteProcess, RemoteProcessError, Session};
use log::*;
use terminal_size::{terminal_size, Height, Width};
use termwiz::{
    caps::Capabilities,
    input::{InputEvent, KeyCodeEncodeModes},
    terminal::{new_terminal, Terminal},
};
use tokio::{io, time::Duration};

#[derive(Debug, Display, Error, From)]
pub enum Error {
    #[display(fmt = "Process failed with exit code: {}", _0)]
    BadProcessExit(#[error(not(source))] i32),
    Io(io::Error),
    RemoteProcess(RemoteProcessError),
}

impl ExitCodeError for Error {
    fn is_silent(&self) -> bool {
        match self {
            Self::RemoteProcess(x) => x.is_silent(),
            _ => false,
        }
    }

    fn to_exit_code(&self) -> ExitCode {
        match self {
            Self::BadProcessExit(x) => ExitCode::Custom(*x),
            Self::Io(x) => x.to_exit_code(),
            Self::RemoteProcess(x) => x.to_exit_code(),
        }
    }
}

pub fn run(cmd: ShellSubcommand, opt: CommonOpt) -> Result<(), Error> {
    let rt = tokio::runtime::Runtime::new()?;

    rt.block_on(async { run_async(cmd, opt).await })
}

async fn run_async(cmd: ShellSubcommand, opt: CommonOpt) -> Result<(), Error> {
    let method = cmd.method;
    let timeout = opt.to_timeout_duration();
    let ssh_connection = cmd.ssh_connection.clone();
    let session_input = cmd.session;
    let session_file = cmd.session_data.session_file.clone();
    let session_socket = cmd.session_data.session_socket.clone();

    CommandRunner {
        method,
        ssh_connection,
        session_input,
        session_file,
        session_socket,
        timeout,
    }
    .run(
        |session, _, lsp_data| Box::pin(start(cmd, session, lsp_data)),
        Error::Io,
    )
    .await
}

async fn start(
    cmd: ShellSubcommand,
    session: Session,
    lsp_data: Option<LspMsg>,
) -> Result<(), Error> {
    let mut proc = RemoteProcess::spawn(
        session.clone_channel(),
        cmd.cmd.unwrap_or_else(|| "/bin/sh".to_string()),
        cmd.persist,
        terminal_size().map(|(Width(cols), Height(rows))| PtySize::from_rows_and_cols(rows, cols)),
    )
    .await?;

    // If we also parsed an LSP's initialize request for its session, we want to forward
    // it along in the case of a process call
    if let Some(data) = lsp_data {
        proc.stdin
            .as_mut()
            .unwrap()
            .write(data.to_string().as_bytes())
            .await?;
    }

    // Create a new terminal in raw mode
    let mut terminal = new_terminal(
        Capabilities::new_from_env().map_err(|x| io::Error::new(io::ErrorKind::Other, x))?,
    )
    .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;
    terminal
        .set_raw_mode()
        .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;

    let mut stdin = proc.stdin.take().unwrap();
    let resizer = proc.clone_resizer();
    tokio::spawn(async move {
        while let Ok(input) = terminal.poll_input(Some(Duration::new(0, 0))) {
            match input {
                Some(InputEvent::Key(ev)) => {
                    if let Ok(input) = ev.key.encode(
                        ev.modifiers,
                        KeyCodeEncodeModes {
                            enable_csi_u_key_encoding: false,
                            application_cursor_keys: false,
                            newline_mode: false,
                        },
                    ) {
                        if let Err(x) = stdin.write_str(input).await {
                            error!("Failed to write to stdin of remote process: {}", x);
                            break;
                        }
                    }
                }
                Some(InputEvent::Resized { cols, rows }) => {
                    if let Err(x) = resizer
                        .resize(PtySize::from_rows_and_cols(rows as u16, cols as u16))
                        .await
                    {
                        error!("Failed to resize remote process: {}", x);
                        break;
                    }
                }
                Some(_) => continue,
                None => tokio::time::sleep(Duration::from_millis(1)).await,
            }
        }
    });

    // Now, map the remote shell's stdout/stderr to our own process,
    // while stdin is handled by the task above
    let link = RemoteProcessLink::from_remote_pipes(
        None,
        proc.stdout.take().unwrap(),
        proc.stderr.take().unwrap(),
    );

    // Continually loop to check for terminal resize changes while the process is still running
    let (success, exit_code) = proc.wait().await?;

    // Shut down our link
    link.shutdown().await;

    if !success {
        if let Some(code) = exit_code {
            return Err(Error::BadProcessExit(code));
        } else {
            return Err(Error::BadProcessExit(1));
        }
    }

    Ok(())
}
