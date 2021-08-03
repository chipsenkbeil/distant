use crate::{
    opt::{CommonOpt, Mode, SessionSubcommand},
    session::SessionFile,
};
use tokio::io;

pub fn run(cmd: SessionSubcommand, _opt: CommonOpt) -> Result<(), io::Error> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        match cmd {
            SessionSubcommand::Clear => SessionFile::clear().await,
            SessionSubcommand::Exists => {
                if SessionFile::exists() {
                    Ok(())
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::NotFound,
                        "No session available",
                    ))
                }
            }
            SessionSubcommand::Info { mode } => {
                let session = SessionFile::load()
                    .await
                    .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;
                match mode {
                    Mode::Json => {
                        println!(
                            "{}",
                            serde_json::to_string(&serde_json::json!({
                                "host": session.host,
                                "port": session.port,
                            }))
                            .unwrap()
                        );
                    }
                    Mode::Shell => {
                        println!("Host: {}", session.host);
                        println!("Port: {}", session.port);
                    }
                }
                Ok(())
            }
        }
    })
}
