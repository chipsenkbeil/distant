use crate::opt::ListenSubcommand;
use derive_more::{Display, Error, From};
use orion::aead;
use std::string::FromUtf8Error;
use tokio::io;

pub type Result = std::result::Result<(), Error>;

#[derive(Debug, Display, Error, From)]
pub enum Error {
    IoError(io::Error),
    Utf8Error(FromUtf8Error),
}

pub async fn run(cmd: ListenSubcommand) -> Result {
    let port = cmd.port;
    let key = aead::SecretKey::default();

    // TODO: We have to share the key in some manner (maybe use k256 to arrive at the same key?)
    //       For now, we do what mosh does and print out the key knowing that this is shared over
    //       ssh, which should provide security
    print!(
        "DISTANT DATA {} {}",
        port,
        hex::encode(key.unprotected_as_bytes())
    );

    // MAC -> Decrypt
    Ok(())
}
