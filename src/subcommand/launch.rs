use crate::{
    opt::{CommonOpt, LaunchSubcommand, SessionSharing},
    session::{Session, SessionFile},
};
use derive_more::{Display, Error, From};
use hex::FromHexError;
use orion::errors::UnknownCryptoError;
use std::string::FromUtf8Error;
use tokio::{io, process::Command};

#[derive(Debug, Display, Error, From)]
pub enum Error {
    #[display(fmt = "Missing data for session")]
    MissingSessionData,

    BadKey(UnknownCryptoError),
    HexError(FromHexError),
    IoError(io::Error),
    Utf8Error(FromUtf8Error),
}

pub fn run(cmd: LaunchSubcommand, opt: CommonOpt) -> Result<(), Error> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async { run_async(cmd, opt).await })
}

async fn run_async(cmd: LaunchSubcommand, _opt: CommonOpt) -> Result<(), Error> {
    let remote_command = format!(
        "{} listen --daemon --host {} {}",
        cmd.remote_program,
        cmd.bind_server,
        cmd.extra_server_args.unwrap_or_default(),
    );
    let ssh_command = format!(
        "{} -o StrictHostKeyChecking=no ssh://{}@{}:{} {} {}",
        cmd.ssh_program,
        cmd.username,
        cmd.host.as_str(),
        cmd.port,
        cmd.identity_file
            .map(|f| format!("-i {}", f.as_path().display()))
            .unwrap_or_default(),
        remote_command.trim(),
    );
    let out = Command::new("sh")
        .arg("-c")
        .arg(ssh_command)
        .output()
        .await?;

    // If our attempt to run the program via ssh failed, report it
    if !out.status.success() {
        return Err(Error::from(io::Error::new(
            io::ErrorKind::Other,
            String::from_utf8(out.stderr)?.trim().to_string(),
        )));
    }

    // Parse our output for the specific session line
    // NOTE: The host provided on this line isn't valid, so we fill it in with our actual host
    let out = String::from_utf8(out.stdout)?.trim().to_string();
    let mut session = out
        .lines()
        .find_map(|line| line.parse::<Session>().ok())
        .ok_or(Error::MissingSessionData)?;
    session.host = cmd.host;

    // Handle sharing resulting session in different ways
    // NOTE: Environment is unreachable here as we disallow it from the defined options since
    //       there is no way to set the shell's environment variables, only this running process
    match cmd.session {
        SessionSharing::Environment => unreachable!(),
        SessionSharing::File => SessionFile::from(session).save().await?,
        SessionSharing::Pipe => println!("{}", session.to_unprotected_string()),
    }

    Ok(())
}
