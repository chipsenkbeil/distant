use crate::{
    opt::{CommonOpt, LaunchSubcommand, SessionSharing},
    session::{Session, SessionFile},
};
use derive_more::{Display, Error, From};
use hex::FromHexError;
use orion::{aead::SecretKey, errors::UnknownCryptoError};
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

    // Parse our output for the specific info line
    let out = String::from_utf8(out.stdout)?.trim().to_string();
    let result = out
        .lines()
        .find_map(|line| {
            let tokens: Vec<&str> = line.split(' ').take(4).collect();
            let is_data_line = tokens.len() == 4 && tokens[0] == "DISTANT" && tokens[1] == "DATA";
            match tokens[2].parse::<u16>() {
                Ok(port) if is_data_line => {
                    let key = hex::decode(tokens[3])
                        .map_err(Error::from)
                        .and_then(|bytes| SecretKey::from_slice(&bytes).map_err(Error::from));
                    match key {
                        Ok(key) => Some(Ok((port, key))),
                        Err(x) => Some(Err(x)),
                    }
                }
                _ => None,
            }
        })
        .unwrap_or(Err(Error::MissingSessionData));

    // Write a session file containing our data for use in subsequent calls
    let (port, auth_key) = result?;
    let session = Session {
        host: cmd.host,
        port,
        auth_key,
    };

    // Handle sharing resulting session in different ways
    // NOTE: Environment is unreachable here as we disallow it from the defined options since
    //       there is no way to set the shell's environment variables, only this running process
    match cmd.session {
        SessionSharing::Environment => unreachable!(),
        SessionSharing::File => SessionFile::from(session).save().await?,
        SessionSharing::Pipe => println!("{}", session.to_unprotected_string().await?),
    }

    Ok(())
}
