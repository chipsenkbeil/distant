use crate::{
    data::{Request, Response},
    net::{Client, TransportError},
    opt::{CommonOpt, ExecuteFormat, ExecuteSubcommand},
    utils::{Session, SessionError},
};
use derive_more::{Display, Error, From};
use tokio::io;

#[derive(Debug, Display, Error, From)]
pub enum Error {
    IoError(io::Error),
    SessionError(SessionError),
    TransportError(TransportError),
}

pub fn run(cmd: ExecuteSubcommand, opt: CommonOpt) -> Result<(), Error> {
    let rt = tokio::runtime::Runtime::new()?;

    rt.block_on(async { run_async(cmd, opt).await })
}

async fn run_async(cmd: ExecuteSubcommand, _opt: CommonOpt) -> Result<(), Error> {
    let session = Session::load().await?;
    let client = Client::connect(session).await?;

    let req = Request::from(cmd.operation);

    let res = client.send(req).await?;
    let res_string = match cmd.format {
        ExecuteFormat::Json => serde_json::to_string(&res)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?,
        ExecuteFormat::Shell => format!("{:?}", res),
    };
    println!("{}", res_string);

    // TODO: Process result to determine if we want to create a watch stream and continue
    //       to examine results

    Ok(())
}
