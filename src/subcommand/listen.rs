use crate::{
    exit::{ExitCode, ExitCodeError},
    opt::{CommonOpt, ConvertToIpAddrError, ListenSubcommand},
};
use derive_more::{Display, Error, From};
use distant_core::DistantServer;
use fork::{daemon, Fork};
use log::*;
use tokio::{io, task::JoinError};

#[derive(Debug, Display, Error, From)]
pub enum Error {
    ConvertToIpAddrError(ConvertToIpAddrError),
    ForkError,
    IoError(io::Error),
    JoinError(JoinError),
}

impl ExitCodeError for Error {
    fn to_exit_code(&self) -> ExitCode {
        match self {
            Self::ConvertToIpAddrError(_) => ExitCode::NoHost,
            Self::ForkError => ExitCode::OsErr,
            Self::IoError(x) => x.to_exit_code(),
            Self::JoinError(_) => ExitCode::Software,
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
                if let Err(_) = fork::close_fd() {
                    return Err(Error::ForkError);
                }
            }
            Err(_) => return Err(Error::ForkError),
        }
    } else {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(async { run_async(cmd, opt, false).await })?;
    }

    Ok(())
}

async fn run_async(cmd: ListenSubcommand, _opt: CommonOpt, is_forked: bool) -> Result<(), Error> {
    let addr = cmd.host.to_ip_addr(cmd.use_ipv6)?;
    let socket_addrs = cmd.port.make_socket_addrs(addr);
    let shutdown_after = cmd.to_shutdown_after_duration();

    // If specified, change the current working directory of this program
    if let Some(path) = cmd.current_dir.as_ref() {
        debug!("Setting current directory to {:?}", path);
        std::env::set_current_dir(path)?;
    }

    // Bind & start our server
    let server = DistantServer::bind(
        addr,
        cmd.port,
        shutdown_after,
        cmd.max_msg_capacity as usize,
    )
    .await?;

    // Print information about port, key, etc.
    println!(
        "DISTANT DATA -- {} {}",
        server.port(),
        server.to_unprotected_hex_auth_key()
    );

    // For the child, we want to fully disconnect it from pipes, which we do now
    if is_forked {
        if let Err(_) = fork::close_fd() {
            return Err(Error::ForkError);
        }
    }

    // Let our server run to completion
    server.wait().await?;

    Ok(())
}
