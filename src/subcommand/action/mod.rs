use crate::{
    data::{Request, ResponsePayload},
    net::{Client, TransportError},
    opt::{ActionSubcommand, CommonOpt, Mode, SessionInput},
    session::{Session, SessionFile},
};
use derive_more::{Display, Error, From};
use log::*;
use tokio::io;

pub(crate) mod inner;

#[derive(Debug, Display, Error, From)]
pub enum Error {
    IoError(io::Error),
    TransportError(TransportError),

    #[display(fmt = "Non-interactive but no operation supplied")]
    MissingOperation,
}

pub fn run(cmd: ActionSubcommand, opt: CommonOpt) -> Result<(), Error> {
    let rt = tokio::runtime::Runtime::new()?;

    rt.block_on(async { run_async(cmd, opt).await })
}

async fn run_async(cmd: ActionSubcommand, _opt: CommonOpt) -> Result<(), Error> {
    let session = match cmd.session {
        SessionInput::Environment => Session::from_environment()?,
        SessionInput::File => SessionFile::load().await?.into(),
        SessionInput::Pipe => Session::from_stdin()?,
    };

    let mut client = Client::connect(session).await?;

    if !cmd.interactive && cmd.operation.is_none() {
        return Err(Error::MissingOperation);
    }

    // Special conditions for continuing to process responses
    let mut is_proc_req = false;
    let mut proc_id = 0;

    if let Some(req) = cmd.operation.map(Request::from) {
        is_proc_req = req.payload.is_proc_run();

        trace!("Client sending request: {:?}", req);
        let res = client.send(req).await?;

        // Store the spawned process id for using in sending stdin (if we spawned a proc)
        proc_id = match &res.payload {
            ResponsePayload::ProcStart { id } => *id,
            _ => 0,
        };

        inner::format_response(cmd.mode, res)?.print();
    }

    // If we are executing a process, we want to continue interacting via stdin and receiving
    // results via stdout/stderr
    //
    // If we are interactive, we want to continue looping regardless
    if is_proc_req || cmd.interactive {
        let config = match cmd.mode {
            Mode::Json => inner::LoopConfig::Json,
            Mode::Shell if cmd.interactive => inner::LoopConfig::Shell,
            Mode::Shell => inner::LoopConfig::Proc { id: proc_id },
        };
        inner::interactive_loop(client, config).await?;
    }

    Ok(())
}
