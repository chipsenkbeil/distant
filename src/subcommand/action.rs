use crate::{
    exit::{ExitCode, ExitCodeError},
    link::RemoteProcessLink,
    opt::{ActionSubcommand, CommonOpt, Format},
    output::ResponseOut,
    session::CliSession,
    subcommand::CommandRunner,
    utils,
};
use derive_more::{Display, Error, From};
use distant_core::{
    ChangeKindSet, LspMsg, RemoteProcess, RemoteProcessError, Request, RequestData, Response,
    ResponseData, Session, TransportError, WatchError, Watcher,
};
use tokio::{io, time::Duration};

#[derive(Debug, Display, Error, From)]
pub enum Error {
    #[display(fmt = "Process failed with exit code: {}", _0)]
    BadProcessExit(#[error(not(source))] i32),
    Io(io::Error),
    #[display(fmt = "Non-interactive but no operation supplied")]
    MissingOperation,
    OperationFailed,
    RemoteProcess(RemoteProcessError),
    Transport(TransportError),
    Watch(WatchError),
}

impl ExitCodeError for Error {
    fn is_silent(&self) -> bool {
        match self {
            Self::BadProcessExit(_) | Self::OperationFailed => true,
            Self::RemoteProcess(x) => x.is_silent(),
            _ => false,
        }
    }

    fn to_exit_code(&self) -> ExitCode {
        match self {
            Self::BadProcessExit(x) => ExitCode::Custom(*x),
            Self::Io(x) => x.to_exit_code(),
            Self::MissingOperation => ExitCode::Usage,
            Self::OperationFailed => ExitCode::Software,
            Self::RemoteProcess(x) => x.to_exit_code(),
            Self::Transport(x) => x.to_exit_code(),
            Self::Watch(x) => x.to_exit_code(),
        }
    }
}

pub fn run(cmd: ActionSubcommand, opt: CommonOpt) -> Result<(), Error> {
    let rt = tokio::runtime::Runtime::new()?;

    rt.block_on(async { run_async(cmd, opt).await })
}

async fn run_async(cmd: ActionSubcommand, opt: CommonOpt) -> Result<(), Error> {
    let method = cmd.method;
    let ssh_connection = cmd.ssh_connection.clone();
    let session_input = cmd.session;
    let timeout = opt.to_timeout_duration();
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
        |session, timeout, lsp_data| Box::pin(start(cmd, session, timeout, lsp_data)),
        Error::Io,
    )
    .await
}

async fn start(
    cmd: ActionSubcommand,
    mut session: Session,
    timeout: Duration,
    lsp_data: Option<LspMsg>,
) -> Result<(), Error> {
    let is_shell_format = matches!(cmd.format, Format::Shell);

    match (cmd.interactive, cmd.operation) {
        // Watch request w/ shell format is specially handled and we ignore interactive as
        // watch will run and wait
        (
            _,
            Some(RequestData::Watch {
                path,
                recursive,
                only,
                except,
            }),
        ) if is_shell_format => {
            let mut watcher = Watcher::watch(
                session.into_channel(),
                path,
                recursive,
                only.into_iter().collect::<ChangeKindSet>(),
                except.into_iter().collect::<ChangeKindSet>(),
            )
            .await?;

            // Continue to receive and process changes
            while let Some(change) = watcher.next().await {
                // TODO: Provide a cleaner way to print just a change
                let res = Response::new("", 0, vec![ResponseData::Changed(change)]);
                ResponseOut::new(cmd.format, res)?.print()
            }

            Ok(())
        }

        // ProcSpawn request w/ shell format is specially handled and we ignore interactive as
        // the stdin will be used for sending ProcStdin to remote process
        (_, Some(RequestData::ProcSpawn { cmd, persist, pty })) if is_shell_format => {
            let mut proc = RemoteProcess::spawn(session.clone_channel(), cmd, persist, pty).await?;

            // If we also parsed an LSP's initialize request for its session, we want to forward
            // it along in the case of a process call
            if let Some(data) = lsp_data {
                proc.stdin.as_mut().unwrap().write(data.to_string()).await?;
            }

            // Now, map the remote process' stdin/stdout/stderr to our own process
            let link = RemoteProcessLink::from_remote_pipes(
                proc.stdin.take(),
                proc.stdout.take().unwrap(),
                proc.stderr.take().unwrap(),
            );

            // Drop main session as the singular remote process will now manage stdin/stdout/stderr
            // NOTE: Without this, severing stdin when from this side would not occur as we would
            //       continue to maintain a second reference to the remote connection's input
            //       through the primary session
            drop(session);

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
            let cli_session = CliSession::new_for_stdin(utils::new_tenant(), session, cmd.format);
            cli_session.wait().await?;

            Ok(())
        }

        // Not interactive and no operation given
        (false, None) => Err(Error::MissingOperation),
    }
}
