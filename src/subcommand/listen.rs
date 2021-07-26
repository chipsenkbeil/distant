use crate::opt::ListenSubcommand;
use derive_more::{Display, Error, From};
use fork::{daemon, Fork};
use orion::aead::SecretKey;
use std::string::FromUtf8Error;
use tokio::io;

pub type Result = std::result::Result<(), Error>;

#[derive(Debug, Display, Error, From)]
pub enum Error {
    ForkError,
    IoError(io::Error),
    Utf8Error(FromUtf8Error),
}

pub fn run(cmd: ListenSubcommand) -> Result {
    // TODO: Determine actual port bound to pre-fork if possible...
    //
    // 1. See if we can bind to a tcp port and then fork
    // 2. If not, we can still output to stdout in the child process (see publish_data); so,
    //    would just bind early in the child process
    let port = cmd.port;
    let key = SecretKey::default();

    if cmd.daemon {
        // NOTE: We keep the stdin, stdout, stderr open so we can print out the pid with the parent
        match daemon(false, true) {
            Ok(Fork::Child) => {
                publish_data(port, &key);

                // For the child, we want to fully disconnect it from pipes, which we do now
                if let Err(_) = fork::close_fd() {
                    return Err(Error::ForkError);
                }

                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(async { run_async(cmd).await })?;
            }
            Ok(Fork::Parent(pid)) => eprintln!("[distant detached, pid = {}]", pid),
            Err(_) => return Err(Error::ForkError),
        }
    } else {
        publish_data(port, &key);

        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(async { run_async(cmd).await })?;
    }

    // MAC -> Decrypt
    Ok(())
}

async fn run_async(_cmd: ListenSubcommand) -> Result {
    // TODO: Implement server logic
    Ok(())
}

fn publish_data(port: u16, key: &SecretKey) {
    // TODO: We have to share the key in some manner (maybe use k256 to arrive at the same key?)
    //       For now, we do what mosh does and print out the key knowing that this is shared over
    //       ssh, which should provide security
    println!(
        "DISTANT DATA {} {}",
        port,
        hex::encode(key.unprotected_as_bytes())
    );
}
