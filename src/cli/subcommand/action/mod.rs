use crate::{
    cli::{
        opt::{ActionSubcommand, CommonOpt, Format, SessionInput},
        ExitCode, ExitCodeError,
    },
    core::{
        client::{LspData, Session, SessionInfo, SessionInfoFile},
        data::{Request, RequestData, ResponseData},
        net::{DataStream, TransportError},
    },
};
use derive_more::{Display, Error, From};
use log::*;
use tokio::{io, time::Duration};

pub(crate) mod inner;

#[derive(Debug, Display, Error, From)]
pub enum Error {
    IoError(io::Error),
    TransportError(TransportError),

    #[display(fmt = "Non-interactive but no operation supplied")]
    MissingOperation,
}

impl ExitCodeError for Error {
    fn to_exit_code(&self) -> ExitCode {
        match self {
            Self::IoError(x) => x.to_exit_code(),
            Self::TransportError(x) => x.to_exit_code(),
            Self::MissingOperation => ExitCode::Usage,
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
    // TODO: Because lsp is being handled in a separate action, we should fail if we get
    //       a session type of lsp for a regular action
    match (cmd.interactive, cmd.operation) {
        // ProcRun request is specially handled and we ignore interactive as
        // the stdin will be used for sending ProcStdin to remote process
        (_, Some(RequestData::ProcRun { cmd, args })) => {}

        // All other requests without interactive are oneoffs
        (false, Some(req)) => {
            let res = session.send_timeout(req, timeout).await?;
        }

        // Interactive mode will send an optional first request and then continue
        // to read stdin to send more
        (true, maybe_req) => {}

        // Not interactive and no operation given
        (false, None) => Err(Error::MissingOperation),
    }

    // 1. Determine what type of engagement we're doing
    //     a. Oneoff connection, request, response
    //     b. ProcRun where we take over stdin, stdout, stderr to provide a remote
    //        process experience
    //     c. Lsp where we do the ProcRun stuff, but translate stdin before sending and
    //        stdout before outputting
    //     d. Interactive program
    //
    // 2. If we have a queued up operation, we need to perform it
    //    a. For oneoff, this is the request of the oneoff
    //    b. For Procrun, this is the request that starts the process
    //    c. For Lsp, this is the request that starts the process
    //    d. For interactive, this is an optional first request
    //
    // 3. If we are using LSP session mode, then we want to send the
    //    ProcStdin request after our optional queued up operation
    //    a. For oneoff, this doesn't make sense and we should fail
    //    b. For ProcRun, we do this after the ProcStart
    //    c. For Lsp, we do this after the ProcStart
    //    d. For interactive, this doesn't make sense as we only support
    //       JSON and shell command input, not LSP input, so this would
    //       fail and we should fail early
    //
    // ** LSP would be its own action, which means we want to abstract the logic that feeds
    //    into this start method such that it can also be used with lsp action

    // Make up a tenant name
    let tenant = utils::new_tenant();

    // Special conditions for continuing to process responses
    let mut is_proc_req = false;
    let mut proc_id = 0;

    if let Some(req) = cmd
        .operation
        .map(|payload| Request::new(tenant.as_str(), vec![payload]))
    {
        // NOTE: We know that there is a single payload entry, so it's all-or-nothing
        is_proc_req = req.payload.iter().any(|x| x.is_proc_run());

        debug!("Client sending request: {:?}", req);
        let res = session.send_timeout(req, timeout).await?;

        // Store the spawned process id for using in sending stdin (if we spawned a proc)
        // NOTE: We can assume that there is a single payload entry in response to our single
        //       entry in our request
        if let Some(ResponseData::ProcStart { id }) = res.payload.first() {
            proc_id = *id;
        }

        inner::format_response(cmd.format, res)?.print();

        // If we also parsed an LSP's initialize request for its session, we want to forward
        // it along in the case of a process call
        //
        // TODO: Do we need to do this somewhere else to apply to all possible ways an LSP
        //       could be started?
        if let Some(data) = lsp_data {
            session
                .fire_timeout(
                    Request::new(
                        tenant.as_str(),
                        vec![RequestData::ProcStdin {
                            id: proc_id,
                            data: data.to_string(),
                        }],
                    ),
                    timeout,
                )
                .await?;
        }
    }

    // If we are executing a process, we want to continue interacting via stdin and receiving
    // results via stdout/stderr
    //
    // If we are interactive, we want to continue looping regardless
    if is_proc_req || cmd.interactive {
        let config = match cmd.format {
            Format::Json => inner::LoopConfig::Json,
            Format::Shell if cmd.interactive => inner::LoopConfig::Shell,
            Format::Shell => inner::LoopConfig::Proc { id: proc_id },
        };
        inner::interactive_loop(client, tenant, config).await?;
    }

    Ok(())
}
