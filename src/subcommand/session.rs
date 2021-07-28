use crate::{
    opt::{CommonOpt, SessionSubcommand},
    utils::Session,
};
use tokio::io;

pub fn run(cmd: SessionSubcommand, _opt: CommonOpt) -> Result<(), io::Error> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        match cmd {
            SessionSubcommand::Clear => Session::clear().await,
        }
    })
}
