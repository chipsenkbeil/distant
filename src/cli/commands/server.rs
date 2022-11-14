use crate::{
    config::{BindAddress, ServerConfig, ServerListenConfig},
    CliError, CliResult,
};
use anyhow::Context;
use clap::Subcommand;
use distant_core::net::common::authentication::Verifier;
use distant_core::net::common::{Host, SecretKey32};
use distant_core::net::server::{Server, ServerConfig as NetServerConfig, ServerRef};
use distant_core::{DistantApiServerHandler, DistantSingleKeyCredentials};
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

        /// If specified, will send output to the specified named pipe (internal usage)
        #[cfg(windows)]
        #[clap(long, help = None, long_help = None)]
        output_to_local_pipe: Option<std::ffi::OsString>,
    },
}

impl ServerSubcommand {
    pub fn run(self, config: ServerConfig) -> CliResult {
        match &self {
            Self::Listen { daemon, .. } if *daemon => Self::run_daemon(self, config),
            Self::Listen { .. } => {
                let rt = tokio::runtime::Runtime::new().context("Failed to start up runtime")?;
                rt.block_on(Self::async_run(self, config, false))
            }
        }
    }

    #[cfg(windows)]
    fn run_daemon(self, _config: ServerConfig) -> CliResult {
        use crate::cli::Spawner;
        use distant_core::net::common::{Listener, TransportExt, WindowsPipeListener};
        use std::ffi::OsString;
        let rt = tokio::runtime::Runtime::new().context("Failed to start up runtime")?;
        rt.block_on(async {
            let name = format!("distant_{}_{}", std::process::id(), rand::random::<u16>());
            let mut listener = WindowsPipeListener::bind_local(name.as_str())
                .with_context(|| "Failed to bind to local named pipe {name:?}")?;

            let pid = Spawner::spawn_running_background(vec![
                OsString::from("--output-to-local-pipe"),
                OsString::from(name),
            ])
            .context("Failed to spawn background process")?;
            println!("[distant server detached, pid = {}]", pid);

            // Wait to receive a connection from the above process
            let mut transport = listener.accept().await.context(
                "Failed to receive connection from background process to send credentials",
            )?;

            // Get the credentials and print them
            let mut s = String::new();
            let n = transport
                .read_to_string(&mut s)
                .await
                .context("Failed to receive credentials")?;
            if n == 0 {
                anyhow::bail!("No credentials received from spawned server");
            }
            let credentials = s[..n]
                .trim()
                .parse::<DistantSingleKeyCredentials>()
                .context("Failed to parse server credentials")?;

            println!("\r");
            println!("{}", credentials);
            println!("\r");
            io::stdout()
                .flush()
                .context("Failed to print server credentials")?;
            Ok(())
        })
        .map_err(CliError::Error)
    }

    #[cfg(unix)]
    fn run_daemon(self, config: ServerConfig) -> CliResult {
        use fork::{daemon, Fork};

        // NOTE: We keep the stdin, stdout, stderr open so we can print out the pid with the parent
        debug!("Forking process");
        match daemon(true, true) {
            Ok(Fork::Child) => {
                let rt = tokio::runtime::Runtime::new().context("Failed to start up runtime")?;
                rt.block_on(async { Self::async_run(self, config, true).await })?;
                Ok(())
            }
            Ok(Fork::Parent(pid)) => {
                println!("[distant server detached, pid = {}]", pid);
                if fork::close_fd().is_err() {
                    Err(CliError::Error(anyhow::anyhow!("Fork failed to close fd")))
                } else {
                    Ok(())
                }
            }
            Err(_) => Err(CliError::Error(anyhow::anyhow!("Fork failed"))),
        }
    }

    async fn async_run(self, config: ServerConfig, _is_forked: bool) -> CliResult {
        match self {
            Self::Listen {
                config: listen_config,
                key_from_stdin,
                #[cfg(windows)]
                output_to_local_pipe,
                ..
            } => {
                macro_rules! get {
                    (@flag $field:ident) => {{
                        config.listen.$field || listen_config.$field
                    }};
                    ($field:ident) => {{
                        config.listen.$field.or(listen_config.$field)
                    }};
                }

                let host = get!(host).unwrap_or(BindAddress::Any);
                trace!("Starting server using unresolved host '{host}'");
                let addr = host.resolve(get!(@flag use_ipv6)).await?;

                // If specified, change the current working directory of this program
                if let Some(path) = get!(current_dir) {
                    debug!("Setting current directory to {:?}", path);
                    std::env::set_current_dir(path)
                        .context("Failed to set new current directory")?;
                }

                // Bind & start our server
                let key = if key_from_stdin {
                    debug!("Reading secret key from stdin");
                    let mut buf = [0u8; 32];
                    io::stdin()
                        .read_exact(&mut buf)
                        .context("Failed to read secret key from stdin")?;
                    SecretKey32::from(buf)
                } else {
                    SecretKey32::default()
                };

                debug!(
                    "Starting local API server, binding to {} {}",
                    addr,
                    match get!(port) {
                        Some(range) => format!("with port in range {}", range),
                        None => "using an ephemeral port".to_string(),
                    }
                );
                let handler = DistantApiServerHandler::local(NetServerConfig {
                    shutdown: get!(shutdown).unwrap_or_default(),
                    ..Default::default()
                })
                .context("Failed to create local distant api")?;
                let server = Server::tcp()
                    .handler(handler)
                    .verifier(Verifier::static_key(key.clone()))
                    .start(addr, get!(port).unwrap_or_else(|| 0.into()))
                    .await
                    .with_context(|| {
                        format!(
                            "Failed to start server @ {} with {}",
                            addr,
                            get!(port)
                                .map(|p| format!("port in range {p}"))
                                .unwrap_or_else(|| String::from("ephemeral port"))
                        )
                    })?;

                let credentials = DistantSingleKeyCredentials {
                    host: Host::from(addr),
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
                    io::stdout()
                        .flush()
                        .context("Failed to print credentials")?;
                }

                #[cfg(windows)]
                if let Some(name) = output_to_local_pipe {
                    use distant_core::net::common::{TransportExt, WindowsPipeTransport};
                    let transport = WindowsPipeTransport::connect_local(&name)
                        .await
                        .with_context(|| {
                            format!("Failed to connect to local pipe named {name:?}")
                        })?;
                    transport
                        .write_all(credentials.to_string().as_bytes())
                        .await
                        .context("Failed to send credentials through pipe")?;
                } else {
                    println!("\r");
                    println!("{}", credentials);
                    println!("\r");
                    io::stdout()
                        .flush()
                        .context("Failed to print credentials")?;
                }

                // For the child, we want to fully disconnect it from pipes, which we do now
                #[cfg(unix)]
                if _is_forked && fork::close_fd().is_err() {
                    return Err(CliError::Error(anyhow::anyhow!("Fork failed to close fd")));
                }

                // Let our server run to completion
                server.wait().await.context("Failed to wait on server")?;
                info!("Server is shutting down");
            }
        }

        Ok(())
    }
}
