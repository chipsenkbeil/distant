use crate::{
    cli::CliResult,
    config::{BindAddress, ServerConfig, ServerListenConfig},
};
use clap::Subcommand;
use distant_core::{
    net::{SecretKey32, ServerRef, TcpServerExt, XChaCha20Poly1305Codec},
    DistantApiServer, DistantSingleKeyCredentials,
};
use log::*;
use std::io::{self, Read, Write};

#[derive(Debug, Subcommand)]
pub enum ServerSubcommand {
    /// Listen for incoming requests as a server
    Listen {
        #[clap(flatten)]
        config: ServerListenConfig,

        /// If specified, will fork the process to run as a standalone daemon
        #[clap(long)]
        daemon: bool,

        /// If specified, the server will not generate a key but instead listen on stdin for the next
        /// 32 bytes that it will use as the key instead. Receiving less than 32 bytes before stdin
        /// is closed is considered an error and any bytes after the first 32 are not used for the key
        #[clap(long)]
        key_from_stdin: bool,
    },
}

impl ServerSubcommand {
    pub fn run(self, config: ServerConfig) -> CliResult<()> {
        match &self {
            Self::Listen { daemon, .. } if *daemon => Self::run_daemon(self, config),
            _ => {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(Self::async_run(self, config, false))
            }
        }
    }

    #[cfg(windows)]
    fn run_daemon(self, _config: ServerConfig) -> CliResult<()> {
        use std::{
            os::windows::process::CommandExt,
            process::{Command, Stdio},
        };
        let mut args = std::env::args_os();
        let program = args
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::Other, "Fork failed"))?;

        // Remove daemon argument to to ensure runs in foreground, otherwise we would fork bomb ourselves
        let args = args.filter(|arg| {
            !arg.to_str()
                .map(|s| s.trim().eq_ignore_ascii_case("--daemon"))
                .unwrap_or_default()
        });

        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        // TODO: Can the detached process still communicate to stdout?
        let child = Command::new(program)
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        info!("[distant detached, pid = {}]", child.id());
        Ok(())
    }

    #[cfg(unix)]
    fn run_daemon(self, config: ServerConfig) -> CliResult<()> {
        use fork::{daemon, Fork};

        // NOTE: We keep the stdin, stdout, stderr open so we can print out the pid with the parent
        match daemon(false, true) {
            Ok(Fork::Child) => {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(async { Self::async_run(self, config, true).await })?;
                Ok(())
            }
            Ok(Fork::Parent(pid)) => {
                info!("[distant server detached, pid = {}]", pid);
                if fork::close_fd().is_err() {
                    Err(io::Error::new(io::ErrorKind::Other, "Fork failed to close fd").into())
                } else {
                    Ok(())
                }
            }
            Err(_) => Err(io::Error::new(io::ErrorKind::Other, "Fork failed").into()),
        }
    }

    async fn async_run(self, config: ServerConfig, is_forked: bool) -> CliResult<()> {
        match self {
            Self::Listen {
                config,
                key_from_stdin,
                ..
            } => {
                let addr = config
                    .host
                    .unwrap_or(BindAddress::Any)
                    .resolve(config.use_ipv6)?;

                // If specified, change the current working directory of this program
                if let Some(path) = config.current_dir.as_ref() {
                    debug!("Setting current directory to {:?}", path);
                    std::env::set_current_dir(path)?;
                }

                // Bind & start our server
                let key = if key_from_stdin {
                    let mut buf = [0u8; 32];
                    let _ = io::stdin().read_exact(&mut buf)?;
                    SecretKey32::from(buf)
                } else {
                    SecretKey32::default()
                };

                let codec = XChaCha20Poly1305Codec::new(key.unprotected_as_bytes());

                let server = DistantApiServer::local()?
                    .start(addr, config.port.unwrap_or_else(|| 0.into()), codec)
                    .await?;

                let credentials = DistantSingleKeyCredentials {
                    host: addr.to_string(),
                    port: server.port(),
                    key,
                    username: None,
                };

                // Print information about port, key, etc.
                // NOTE: Following mosh approach of printing to make sure there's no garbage floating around
                println!("\r");
                println!("{}", credentials);
                println!("\r");
                io::stdout().flush()?;

                // For the child, we want to fully disconnect it from pipes, which we do now
                #[cfg(unix)]
                if is_forked && fork::close_fd().is_err() {
                    return Err(
                        io::Error::new(io::ErrorKind::Other, "Fork failed to close fd").into(),
                    );
                }

                // Let our server run to completion
                server.wait().await?;
            }
        }

        Ok(())
    }
}
