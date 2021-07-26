use crate::{opt::ExecuteSubcommand, SESSION_PATH};
use derive_more::{Display, Error, From};
use orion::aead::SecretKey;
use tokio::io;

pub type Result = std::result::Result<(), Error>;

#[derive(Debug, Display, Error, From)]
pub enum Error {
    #[display(fmt = "Invalid key for session")]
    InvalidSessionKey,

    #[display(fmt = "Invalid port for session")]
    InvalidSessionPort,

    IoError(io::Error),

    #[display(fmt = "Missing key for session")]
    MissingSessionKey,

    #[display(fmt = "Missing port for session")]
    MissingSessionPort,

    #[display(fmt = "No session file: {:?}", SESSION_PATH.as_path())]
    NoSessionFile,
}

pub async fn run(_cmd: ExecuteSubcommand) -> Result {
    // Load our session file's port and key
    let (port, key) = {
        let text = tokio::fs::read_to_string(SESSION_PATH.as_path())
            .await
            .map_err(|_| Error::NoSessionFile)?;
        let mut tokens = text.split(' ').take(2);
        let port = tokens
            .next()
            .ok_or(Error::MissingSessionPort)?
            .parse::<u16>()
            .map_err(|_| Error::InvalidSessionPort)?;
        let key = SecretKey::from_slice(
            &hex::decode(tokens.next().ok_or(Error::MissingSessionKey)?.to_string())
                .map_err(|_| Error::InvalidSessionKey)?,
        )
        .map_err(|_| Error::InvalidSessionKey)?;
        (port, key)
    };

    println!(
        "PORT:{}; KEY:{}",
        port,
        hex::encode(key.unprotected_as_bytes())
    );

    // Encrypt -> MAC
    Ok(())
}
