use std::io::{self, Read, Write};

use anyhow::Context;
use distant_core::net::auth::Verifier;
use distant_core::net::common::{Host, SecretKey32};
use distant_core::net::server::{Server, ServerConfig as NetServerConfig, ServerRef};
use distant_core::DistantSingleKeyCredentials;
use log::*;

use crate::options::ServerSubcommand;
use crate::{CliError, CliResult};

pub fn run(cmd: ServerSubcommand) -> CliResult {
    match &cmd {
        ServerSubcommand::Listen { daemon, .. } if *daemon => run_daemon(cmd),
        ServerSubcommand::Listen { .. } => {
            let rt = tokio::runtime::Runtime::new().context("Failed to start up runtime")?;
            rt.block_on(async_run(cmd, false))
        }
    }
}

#[cfg(windows)]
fn run_daemon(_cmd: ServerSubcommand) -> CliResult {
    use std::ffi::OsString;

    use distant_core::net::common::{Listener, TransportExt, WindowsPipeListener};

    use crate::cli::Spawner;
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
        let transport = listener
            .accept()
            .await
            .context("Failed to receive connection from background process to send credentials")?;

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
fn run_daemon(cmd: ServerSubcommand) -> CliResult {
    use fork::{daemon, Fork};

    // NOTE: We keep the stdin, stdout, stderr open so we can print out the pid with the parent
    debug!("Forking process");
    match daemon(true, true) {
        Ok(Fork::Child) => {
            let rt = tokio::runtime::Runtime::new().context("Failed to start up runtime")?;
            rt.block_on(async { async_run(cmd, true).await })?;
            Ok(())
        }
        Ok(Fork::Parent(pid)) => {
            println!("[distant server detached, pid = {pid}]");
            if fork::close_fd().is_err() {
                Err(CliError::Error(anyhow::anyhow!("Fork failed to close fd")))
            } else {
                Ok(())
            }
        }
        Err(_) => Err(CliError::Error(anyhow::anyhow!("Fork failed"))),
    }
}

async fn async_run(cmd: ServerSubcommand, _is_forked: bool) -> CliResult {
    match cmd {
        #[allow(unused_variables)]
        ServerSubcommand::Listen {
            host,
            port,
            use_ipv6,
            shutdown,
            current_dir,
            daemon: _,
            key_from_stdin,
            output_to_local_pipe,
        } => {
            let host = host.into_inner();
            trace!("Starting server using unresolved host '{host}'");
            let addr = host.resolve(use_ipv6).await?;

            // If specified, change the current working directory of this program
            if let Some(path) = current_dir {
                debug!("Setting current directory to {:?}", path);
                std::env::set_current_dir(path).context("Failed to set new current directory")?;
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

            let port = port.into_inner();
            debug!(
                "Starting local API server, binding to {} {}",
                addr,
                if port.is_ephemeral() {
                    format!("with port in range {port}")
                } else {
                    "using an ephemeral port".to_string()
                }
            );
            let handler = distant_local::new_handler(Default::default())
                .context("Failed to create local distant api")?;
            let server = Server::tcp()
                .config(NetServerConfig {
                    shutdown: shutdown.into_inner(),
                    ..Default::default()
                })
                .handler(handler)
                .verifier(Verifier::static_key(key.clone()))
                .start(addr, port)
                .await
                .with_context(|| format!("Failed to start server @ {addr} with {port}"))?;

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
                println!("{credentials}");
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
                    .with_context(|| format!("Failed to connect to local pipe named {name:?}"))?;
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
