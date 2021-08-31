use crate::{
    exit::{ExitCode, ExitCodeError},
    link::RemoteProcessLink,
    opt::{ActionSubcommand, CommonOpt, SessionInput},
    output::ResponseOut,
    session::CliSession,
    utils,
};
use derive_more::{Display, Error, From};
use distant_core::{
    DataStream, LspData, RemoteProcess, RemoteProcessError, Request, RequestData, Session,
    SessionInfo, SessionInfoFile, TransportError,
};
use tokio::{io, time::Duration};

#[derive(Debug, Display, Error, From)]
pub enum Error {
    #[display(fmt = "Process failed with exit code: {}", _0)]
    BadProcessExit(#[error(not(source))] i32),
    IoError(io::Error),
    #[display(fmt = "Non-interactive but no operation supplied")]
    MissingOperation,
    OperationFailed,
    RemoteProcessError(RemoteProcessError),
    TransportError(TransportError),
}

impl ExitCodeError for Error {
    fn is_silent(&self) -> bool {
        match self {
            Self::BadProcessExit(_) | Self::OperationFailed => true,
            Self::RemoteProcessError(x) => x.is_silent(),
            _ => false,
        }
    }

    fn to_exit_code(&self) -> ExitCode {
        match self {
            Self::BadProcessExit(x) => ExitCode::Custom(*x),
            Self::IoError(x) => x.to_exit_code(),
            Self::MissingOperation => ExitCode::Usage,
            Self::OperationFailed => ExitCode::Software,
            Self::RemoteProcessError(x) => x.to_exit_code(),
            Self::TransportError(x) => x.to_exit_code(),
        }
    }
}

pub fn run(cmd: ActionSubcommand, opt: CommonOpt) -> Result<(), Error> {
    let rt = tokio::runtime::Runtime::new()?;

    rt.block_on(async { run_async(cmd, opt).await })
}

async fn run_async(cmd: ActionSubcommand, opt: CommonOpt) -> Result<(), Error> {
    let timeout = opt.to_timeout_duration();

    match cmd.session {
        SessionInput::Environment => {
            start(
                cmd,
                Session::tcp_connect_timeout(SessionInfo::from_environment()?, timeout).await?,
                timeout,
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
                timeout,
                None,
            )
            .await
        }
        SessionInput::Pipe => {
            start(
                cmd,
                Session::tcp_connect_timeout(SessionInfo::from_stdin()?, timeout).await?,
                timeout,
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
                timeout,
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
                timeout,
                None,
            )
            .await
        }
    }
}

async fn start<T>(
    cmd: ActionSubcommand,
    mut session: Session<T>,
    timeout: Duration,
    lsp_data: Option<LspData>,
) -> Result<(), Error>
where
    T: DataStream + 'static,
{
    match (cmd.interactive, cmd.operation) {
        // ProcRun request is specially handled and we ignore interactive as
        // the stdin will be used for sending ProcStdin to remote process
        (_, Some(RequestData::ProcRun { cmd, args })) => {
            let mut proc = RemoteProcess::spawn(utils::new_tenant(), session, cmd, args).await?;

            // If we also parsed an LSP's initialize request for its session, we want to forward
            // it along in the case of a process call
            if let Some(data) = lsp_data {
                proc.stdin.as_mut().unwrap().write(data.to_string()).await?;
            }

            // Now, map the remote process' stdin/stdout/stderr to our own process
            let link = RemoteProcessLink::from_remote_pipes(
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

        // All other requests without interactive are oneoffs
        (false, Some(data)) => {
            let res = session
                .send_timeout(Request::new(utils::new_tenant(), vec![data]), timeout)
                .await?;

            // If we have an error as our payload, then we want to reflect that in our
            // exit code
            let is_err = res.payload.iter().any(|d| d.is_error());

            ResponseOut::new(cmd.format, res)?.print();

            if is_err {
                Err(Error::OperationFailed)
            } else {
                Ok(())
            }
        }

        // Interactive mode will send an optional first request and then continue
        // to read stdin to send more
        (true, maybe_req) => {
            // Send our first request if provided
            if let Some(data) = maybe_req {
                let res = session
                    .send_timeout(Request::new(utils::new_tenant(), vec![data]), timeout)
                    .await?;
                ResponseOut::new(cmd.format, res)?.print();
            }

            // Enter into CLI session where we receive requests on stdin and send out
            // over stdout/stderr
            let cli_session = CliSession::new(utils::new_tenant(), session, cmd.format);
            cli_session.wait().await?;

            Ok(())
        }

        // Not interactive and no operation given
        (false, None) => Err(Error::MissingOperation),
    }
}
