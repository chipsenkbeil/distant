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
use std::{
    ffi::OsString,
    io::{self, Read, Write},
};

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

        /// If specified, will send output to the specified named pipe
        #[cfg(windows)]
        output_to_local_pipe: Option<OsString>,
    },
}

impl ServerSubcommand {
    pub fn run(self, _config: ServerConfig) -> CliResult<()> {
        match &self {
            Self::Listen { daemon, .. } if *daemon => Self::run_daemon(self),
            Self::Listen { .. } => {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(Self::async_run(self, false))
            }
        }
    }

    // #[cfg(wincdows)]
    fn run_daemon(self) -> CliResult<()> {
        use crate::cli::Spawner;
        use distant_core::net::{Listener, WindowsPipeListener};
        use tokio::io::AsyncReadExt;
        let rt = tokio::runtime::Runtime::new()?;
        rt.block_on(async {
            let name = format!("distant_daemon_{}", rand::random::<usize>());
            let mut listener = WindowsPipeListener::bind_local(name.as_str())?;

            let pid = Spawner::spawn_running_background(vec![
                OsString::from("--output-to-local-pipe"),
                OsString::from(name),
            ])?;
            println!("[distant server detached, pid = {}]", pid);

            // Wait to receive a connection from the above process
            let mut transport = listener.accept().await?;

            // Get the credentials and print them
            let mut s = String::new();
            let n = transport.read_to_string(&mut s).await?;
            let credentials = s[..n]
                .trim()
                .parse::<DistantSingleKeyCredentials>()
                .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;

            println!("\r");
            println!("{}", credentials);
            println!("\r");
            io::stdout().flush()?;
            Ok(())
        })
    }

    #[cfg(unix)]
    fn run_daemon(self) -> CliResult<()> {
        use fork::{daemon, Fork};

        // NOTE: We keep the stdin, stdout, stderr open so we can print out the pid with the parent
        debug!("Forking process");
        match daemon(false, true) {
            Ok(Fork::Child) => {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(async { Self::async_run(self, true).await })?;
                Ok(())
            }
            Ok(Fork::Parent(pid)) => {
                println!("[distant server detached, pid = {}]", pid);
                if fork::close_fd().is_err() {
                    Err(io::Error::new(io::ErrorKind::Other, "Fork failed to close fd").into())
                } else {
                    Ok(())
                }
            }
            Err(_) => Err(io::Error::new(io::ErrorKind::Other, "Fork failed").into()),
        }
    }

    async fn async_run(self, is_forked: bool) -> CliResult<()> {
        match self {
            Self::Listen {
                config,
                key_from_stdin,
                #[cfg(windows)]
                output_to_local_pipe,
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
                    debug!("Reading secret key from stdin");
                    let mut buf = [0u8; 32];
                    let _ = io::stdin().read_exact(&mut buf)?;
                    SecretKey32::from(buf)
                } else {
                    SecretKey32::default()
                };

                let codec = XChaCha20Poly1305Codec::new(key.unprotected_as_bytes());

                debug!(
                    "Starting local API server, binding to {} {}",
                    addr,
                    match config.port {
                        Some(range) => format!("with port in range {}", range),
                        None => "using an ephemeral port".to_string(),
                    }
                );
                let server = DistantApiServer::local()?
                    .start(addr, config.port.unwrap_or_else(|| 0.into()), codec)
                    .await?;

                let credentials = DistantSingleKeyCredentials {
                    host: addr.to_string(),
                    port: server.port(),
                    key,
                    username: None,
                };
                info!(
                    "Server listening at {}:{}",
                    credentials.host, credentials.port
                );

                // Print information about port, key, etc.
                // NOTE: Following mosh approach of printing to make sure there's no garbage floating around
                #[cfg(not(windows))]
                {
                    println!("\r");
                    println!("{}", credentials);
                    println!("\r");
                    io::stdout().flush()?;
                }

                #[cfg(windows)]
                if let Some(name) = output_to_local_pipe {
                    use distant_core::net::WindowsPipeTransport;
                    use tokio::io::AsyncWriteExt;
                    let mut transport = WindowsPipeTransport::connect_local(name).await?;
                    transport
                        .write_all(credentials.to_string().as_bytes())
                        .await?;
                } else {
                    println!("\r");
                    println!("{}", credentials);
                    println!("\r");
                    io::stdout().flush()?;
                }

                // For the child, we want to fully disconnect it from pipes, which we do now
                #[cfg(unix)]
                if is_forked && fork::close_fd().is_err() {
                    return Err(
                        io::Error::new(io::ErrorKind::Other, "Fork failed to close fd").into(),
                    );
                }

                // Let our server run to completion
                server.wait().await?;
                info!("Server is shutting down");
            }
        }

        Ok(())
    }
}
