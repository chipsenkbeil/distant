use crate::{
    exit::{ExitCode, ExitCodeError},
    link::RemoteProcessLink,
    opt::{CommonOpt, LspSubcommand},
    subcommand::CommandRunner,
    utils,
};
use derive_more::{Display, Error, From};
use distant_core::{LspData, PtySize, RemoteLspProcess, RemoteProcessError, Session};
use terminal_size::{terminal_size, Height, Width};
use tokio::io;

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

pub fn run(cmd: LspSubcommand, opt: CommonOpt) -> Result<(), Error> {
    let rt = tokio::runtime::Runtime::new()?;

    rt.block_on(async { run_async(cmd, opt).await })
}

async fn run_async(cmd: LspSubcommand, opt: CommonOpt) -> Result<(), Error> {
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
    cmd: LspSubcommand,
    session: Session,
    lsp_data: Option<LspData>,
) -> Result<(), Error> {
    let mut proc = RemoteLspProcess::spawn(
        utils::new_tenant(),
        session.clone_channel(),
        cmd.cmd,
        cmd.args,
        cmd.persist,
        if cmd.pty {
            terminal_size()
                .map(|(Width(width), Height(height))| PtySize::from_rows_and_cols(height, width))
        } else {
            None
        },
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

    // Now, map the remote LSP server's stdin/stdout/stderr to our own process
    let link = RemoteProcessLink::from_remote_lsp_pipes(
        proc.stdin.take(),
        proc.stdout.take().unwrap(),
        proc.stderr.take().unwrap(),
    );

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
