use crate::{
    exit::{ExitCode, ExitCodeError},
    link::RemoteProcessLink,
    opt::{CommonOpt, LspSubcommand, SessionInput},
    utils,
};
use derive_more::{Display, Error, From};
use distant_core::{
    DataStream, LspData, RemoteLspProcess, RemoteProcessError, Session, SessionInfo,
    SessionInfoFile,
};
use tokio::io;

#[derive(Debug, Display, Error, From)]
pub enum Error {
    #[display(fmt = "Process failed with exit code: {}", _0)]
    BadProcessExit(#[error(not(source))] i32),
    IoError(io::Error),
    RemoteProcessError(RemoteProcessError),
}

impl ExitCodeError for Error {
    fn is_silent(&self) -> bool {
        match self {
            Self::RemoteProcessError(x) => x.is_silent(),
            _ => false,
        }
    }

    fn to_exit_code(&self) -> ExitCode {
        match self {
            Self::BadProcessExit(x) => ExitCode::Custom(*x),
            Self::IoError(x) => x.to_exit_code(),
            Self::RemoteProcessError(x) => x.to_exit_code(),
        }
    }
}

pub fn run(cmd: LspSubcommand, opt: CommonOpt) -> Result<(), Error> {
    let rt = tokio::runtime::Runtime::new()?;

    rt.block_on(async { run_async(cmd, opt).await })
}

async fn run_async(cmd: LspSubcommand, opt: CommonOpt) -> Result<(), Error> {
    let timeout = opt.to_timeout_duration();

    match cmd.session {
        SessionInput::Environment => {
            start(
                cmd,
                Session::tcp_connect_timeout(SessionInfo::from_environment()?, timeout).await?,
                None,
            )
            .await
        }
        SessionInput::File => {
            let path = cmd.session_data.session_file.clone();
            start(
                cmd,
                Session::tcp_connect_timeout(
                    SessionInfoFile::load_from(path).await?.into(),
                    timeout,
                )
                .await?,
                None,
            )
            .await
        }
        SessionInput::Pipe => {
            start(
                cmd,
                Session::tcp_connect_timeout(SessionInfo::from_stdin()?, timeout).await?,
                None,
            )
            .await
        }
        SessionInput::Lsp => {
            let mut data =
                LspData::from_buf_reader(&mut std::io::stdin().lock()).map_err(io::Error::from)?;
            let info = data.take_session_info().map_err(io::Error::from)?;
            start(
                cmd,
                Session::tcp_connect_timeout(info, timeout).await?,
                Some(data),
            )
            .await
        }
        #[cfg(unix)]
        SessionInput::Socket => {
            let path = cmd.session_data.session_socket.clone();
            start(
                cmd,
                Session::unix_connect_timeout(path, None, timeout).await?,
                None,
            )
            .await
        }
    }
}

async fn start<T>(
    cmd: LspSubcommand,
    session: Session<T>,
    lsp_data: Option<LspData>,
) -> Result<(), Error>
where
    T: DataStream + 'static,
{
    let mut proc = RemoteLspProcess::spawn(utils::new_tenant(), session, cmd.cmd, cmd.args).await?;

    // If we also parsed an LSP's initialize request for its session, we want to forward
    // it along in the case of a process call
    if let Some(data) = lsp_data {
        proc.stdin
            .as_mut()
            .unwrap()
            .write(&data.to_string())
            .await?;
    }

    // Now, map the remote LSP server's stdin/stdout/stderr to our own process
    let link = RemoteProcessLink::from_remote_lsp_pipes(
        proc.stdin.take().unwrap(),
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
