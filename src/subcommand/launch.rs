use crate::opt::{CommonOpt, LaunchSubcommand};
use derive_more::{Display, Error, From};
use std::string::FromUtf8Error;
use tokio::{io, process::Command};

pub type Result = std::result::Result<(), Error>;

#[derive(Debug, Display, Error, From)]
pub enum Error {
    IoError(io::Error),
    Utf8Error(FromUtf8Error),
}

pub async fn run(cmd: LaunchSubcommand, opt: CommonOpt) -> Result {
    let remote_command = r#"distant listen --print-port"#;
    let ssh_command = format!(
        "ssh -o StrictHostKeyChecking=no ssh://{}@{} {} {}",
        cmd.username,
        cmd.destination,
        cmd.identity_file
            .map(|f| format!("-i {}", f.as_path().display()))
            .unwrap_or_default(),
        remote_command,
    );
    let out = Command::new("sh")
        .arg("-c")
        .arg(ssh_command)
        .output()
        .await?;
    let out = String::from_utf8(out.stdout)?.trim().to_string();
    println!("{}", out);

    Ok(())
}
