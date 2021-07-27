use crate::utils::Session;
use tokio::io;

pub fn run() -> Result<(), io::Error> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async { Session::clear().await })
}
