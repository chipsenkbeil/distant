use crate::{
    exit::{ExitCode, ExitCodeError},
    opt::{CommonOpt, ConvertToIpAddrError, ListenSubcommand},
};
use derive_more::{Display, Error, From};
use distant_core::{
    DistantServerOptions, LocalDistantApi, SecretKey32, UnprotectedToHexKey, XChaCha20Poly1305Codec,
};
use log::*;
use tokio::{
    io::{self, AsyncReadExt, AsyncWriteExt},
    task::JoinError,
};

#[derive(Debug, Display, Error, From)]
pub enum Error {
    BadKey,
    ConverToIpAddr(ConvertToIpAddrError),
    Fork,
    Io(io::Error),
    Join(JoinError),
}

impl ExitCodeError for Error {
    fn to_exit_code(&self) -> ExitCode {
        match self {
            Self::BadKey => ExitCode::Usage,
            Self::ConverToIpAddr(_) => ExitCode::NoHost,
            Self::Fork => ExitCode::OsErr,
            Self::Io(x) => x.to_exit_code(),
            Self::Join(_) => ExitCode::Software,
        }
    }
}

pub fn run(cmd: ListenSubcommand, opt: CommonOpt) -> Result<(), Error> {
    if cmd.foreground {
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(async { run_async(cmd, opt, false).await })?;
    } else {
        run_daemon(cmd, opt)?;
    }

    Ok(())
}

#[cfg(windows)]
fn run_daemon(_cmd: ListenSubcommand, _opt: CommonOpt) -> Result<(), Error> {
    use std::{
        ffi::OsString,
        iter,
        process::{Command, Stdio},
    };
    let mut args = std::env::args_os();
    let program = args.next().ok_or(Error::Fork)?;

    // Ensure that forked server runs in foreground, otherwise we would fork bomb ourselves
    let args = args.chain(iter::once(OsString::from("--foreground")));

    let child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()?;
    info!("[distant detached, pid = {}]", child.id());
    Ok(())
}

#[cfg(unix)]
fn run_daemon(cmd: ListenSubcommand, opt: CommonOpt) -> Result<(), Error> {
    use fork::{daemon, Fork};

    // NOTE: We keep the stdin, stdout, stderr open so we can print out the pid with the parent
    match daemon(false, true) {
        Ok(Fork::Child) => {
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(async { run_async(cmd, opt, true).await })?;
            Ok(())
        }
        Ok(Fork::Parent(pid)) => {
            info!("[distant detached, pid = {}]", pid);
            if fork::close_fd().is_err() {
                Err(Error::Fork)
            } else {
                Ok(())
            }
        }
        Err(_) => Err(Error::Fork),
    }
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
    let key = if cmd.key_from_stdin {
        let mut buf = [0u8; 32];
        let n = io::stdin().read_exact(&mut buf).await?;
        if n < buf.len() {
            return Err(Error::BadKey);
        }
        SecretKey32::from(buf)
    } else {
        SecretKey32::default()
    };
    let key_hex_string = key.unprotected_to_hex_key();
    let codec = XChaCha20Poly1305Codec::from(key);

    let (server, port) = LocalDistantApi::bind(
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
    // NOTE: Following mosh approach of printing to make sure there's no garbage floating around
    println!("\r");
    println!("DISTANT CONNECT -- {} {}", port, key_hex_string);
    println!("\r");
    io::stdout().flush().await?;

    // For the child, we want to fully disconnect it from pipes, which we do now
    #[cfg(unix)]
    if is_forked && fork::close_fd().is_err() {
        return Err(Error::Fork);
    }

    // Let our server run to completion
    server.wait().await?;

    Ok(())
}
