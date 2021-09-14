use crate::{
    exit::{ExitCode, ExitCodeError},
    opt::{CommonOpt, ConvertToIpAddrError, ListenSubcommand},
};
use derive_more::{Display, Error, From};
use distant_core::{
    DistantServer, DistantServerOptions, SecretKey32, UnprotectedToHexKey, XChaCha20Poly1305Codec,
};
use fork::{daemon, Fork};
use log::*;
use std::sync::Arc;
use tokio::{io, task::JoinError};

#[derive(Debug, Display, Error, From)]
pub enum Error {
    ConverToIpAddr(ConvertToIpAddrError),
    Fork,
    Io(io::Error),
    Join(JoinError),
}

impl ExitCodeError for Error {
    fn to_exit_code(&self) -> ExitCode {
        match self {
            Self::ConverToIpAddr(_) => ExitCode::NoHost,
            Self::Fork => ExitCode::OsErr,
            Self::Io(x) => x.to_exit_code(),
            Self::Join(_) => ExitCode::Software,
        }
    }
}

pub fn run(cmd: ListenSubcommand, opt: CommonOpt) -> Result<(), Error> {
    if cmd.daemon {
        // NOTE: We keep the stdin, stdout, stderr open so we can print out the pid with the parent
        match daemon(false, true) {
            Ok(Fork::Child) => {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(async { run_async(cmd, opt, true).await })?;
            }
            Ok(Fork::Parent(pid)) => {
                info!("[distant detached, pid = {}]", pid);
                if fork::close_fd().is_err() {
                    return Err(Error::Fork);
                }
            }
            Err(_) => return Err(Error::Fork),
        }
    } else {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(async { run_async(cmd, opt, false).await })?;
    }

    Ok(())
}

async fn run_async(cmd: ListenSubcommand, _opt: CommonOpt, is_forked: bool) -> Result<(), Error> {
    let addr = cmd.host.to_ip_addr(cmd.use_ipv6)?;
    let shutdown_after = cmd.to_shutdown_after_duration();

    // If specified, change the current working directory of this program
    if let Some(path) = cmd.current_dir.as_ref() {
        debug!("Setting current directory to {:?}", path);
        std::env::set_current_dir(path)?;
    }

    // Bind & start our server
    let key = SecretKey32::default();
    let key_hex_string = key.unprotected_to_hex_key();
    let codec = XChaCha20Poly1305Codec::from(key);

    let (server, port) = DistantServer::bind(
        addr,
        cmd.port,
        codec,
        DistantServerOptions {
            shutdown_after,
            max_msg_capacity: cmd.max_msg_capacity as usize,
        },
    )
    .await?;

    // Print information about port, key, etc.
    println!("DISTANT DATA -- {} {}", port, key_hex_string);

    // For the child, we want to fully disconnect it from pipes, which we do now
    if is_forked && fork::close_fd().is_err() {
        return Err(Error::Fork);
    }

    // Let our server run to completion
    server.wait().await?;

    Ok(())
}
