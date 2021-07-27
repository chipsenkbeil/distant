use crate::{
    data::Response,
    net::{Transport, TransportError},
    opt::{CommonOpt, ExecuteSubcommand},
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
    let mut transport = Transport::connect(session).await?;

    // Send our operation
    transport.send(cmd.operation).await?;

    // Continue to receive and process responses as long as we get them or we decide to end
    loop {
        let response = transport.receive::<Response>().await?;
        println!("RESPONSE: {:?}", response);
    }

    Ok(())
}
