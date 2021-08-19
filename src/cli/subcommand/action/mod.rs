use crate::{
    cli::opt::{ActionSubcommand, CommonOpt, Format, SessionInput},
    core::{
        data::{Request, RequestData, ResponseData},
        lsp::LspData,
        net::{Client, DataStream, TransportError},
        session::{Session, SessionFile},
        utils,
    },
    ExitCode, ExitCodeError,
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
                Client::tcp_connect_timeout(Session::from_environment()?, timeout).await?,
                timeout,
                None,
            )
            .await
        }
        SessionInput::File => {
            let path = cmd.session_data.session_file.clone();
            start(
                cmd,
                Client::tcp_connect_timeout(SessionFile::load_from(path).await?.into(), timeout)
                    .await?,
                timeout,
                None,
            )
            .await
        }
        SessionInput::Pipe => {
            start(
                cmd,
                Client::tcp_connect_timeout(Session::from_stdin()?, timeout).await?,
                timeout,
                None,
            )
            .await
        }
        SessionInput::Lsp => {
            let mut data =
                LspData::from_buf_reader(&mut std::io::stdin().lock()).map_err(io::Error::from)?;
            let session = data.take_session().map_err(io::Error::from)?;
            start(
                cmd,
                Client::tcp_connect_timeout(session, timeout).await?,
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
                Client::unix_connect_timeout(path, None, timeout).await?,
                timeout,
                None,
            )
            .await
        }
    }
}

async fn start<T>(
    cmd: ActionSubcommand,
    mut client: Client<T>,
    timeout: Duration,
    lsp_data: Option<LspData>,
) -> Result<(), Error>
where
    T: DataStream + 'static,
{
    if !cmd.interactive && cmd.operation.is_none() {
        return Err(Error::MissingOperation);
    }

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
        let res = client.send_timeout(req, timeout).await?;

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
            client
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
