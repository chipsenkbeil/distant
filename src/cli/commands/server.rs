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
    pub fn run(self, _config: ServerConfig) -> CliResult<()> {
        match &self {
            Self::Listen { daemon, .. } if *daemon => Self::run_daemon(self),
            Self::Listen { .. } => {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(Self::async_run(self, false))
            }
        }
    }

    #[cfg(windows)]
    fn run_daemon(self) -> CliResult<()> {
        use std::{
            ffi::OsString,
            io::{BufRead, Cursor},
            os::windows::process::CommandExt,
            path::PathBuf,
            process::{Command, Stdio},
        };

        // Get absolute path to powershell
        let powershell = which::which("powershell.exe")
            .map_err(|x| io::Error::new(io::ErrorKind::NotFound, x))?;

        // Get absolute path to our binary
        let program =
            which::which(std::env::current_exe().unwrap_or_else(|_| PathBuf::from("distant.exe")))
                .map_err(|x| io::Error::new(io::ErrorKind::NotFound, x))?;

        // Remove --daemon argument to to ensure runs in foreground,
        // otherwise we would fork bomb ourselves
        //
        // Also, remove first argument (program) since we determined it above
        let cmd = {
            let mut cmd = OsString::new();
            cmd.push("'");
            cmd.push(program.as_os_str());

            let it = std::env::args_os().skip(1).filter(|arg| {
                !arg.to_str()
                    .map(|s| s.trim().eq_ignore_ascii_case("--daemon"))
                    .unwrap_or_default()
            });
            for arg in it {
                cmd.push(" ");
                cmd.push(&arg);
            }

            cmd.push("'");
            cmd
        };

        let args = vec![
            OsString::from(r#"$startup=[wmiclass]"Win32_ProcessStartup""#),
            OsString::from(";"),
            OsString::from(r#"$startup.Properties['ShowWindow'].value=$False"#),
            OsString::from(";"),
            OsString::from("Invoke-WmiMethod"),
            OsString::from("-Class"),
            OsString::from("Win32_Process"),
            OsString::from("-Name"),
            OsString::from("Create"),
            OsString::from("-ArgumentList"),
            {
                let mut arg_list = OsString::new();
                arg_list.push(&cmd);
                arg_list.push(",$null,$startup");
                arg_list
            },
        ];

        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let flags = CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW;

        debug!(
            "Spawning child process: {} {:?}",
            powershell.to_string_lossy(),
            args
        );
        let output = Command::new(powershell.into_os_string())
            .creation_flags(flags)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()?;

        if !output.status.success() {
            Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "Program failed [{}]: {}",
                    output.status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&output.stderr)
                ),
            ))?;
        }

        let stdout = Cursor::new(output.stdout);

        let mut process_id = None;
        let mut return_value = None;
        for line in stdout.lines().filter_map(|l| l.ok()) {
            let line = line.trim();
            if line.starts_with("ProcessId") {
                if let Some((_, id)) = line.split_once(':') {
                    process_id = id.trim().parse::<u64>().ok();
                }
            } else if line.starts_with("ReturnValue") {
                if let Some((_, value)) = line.split_once(':') {
                    return_value = value.trim().parse::<i32>().ok();
                }
            }
        }

        match (return_value, process_id) {
            (Some(0), Some(pid)) => {
                println!("[distant server detached, pid = {}]", pid);
                Ok(())
            }
            (Some(0), None) => Err(io::Error::new(
                io::ErrorKind::Other,
                "Program succeeded, but missing process pid",
            ))?,
            (Some(code), _) => Err(io::Error::new(
                io::ErrorKind::Other,
                format!(
                    "Program failed [{}]: {}",
                    code,
                    String::from_utf8_lossy(&output.stderr)
                ),
            ))?,
            (None, _) => Err(io::Error::new(io::ErrorKind::Other, "Missing return value"))?,
        }
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
                info!("Server is shutting down");
            }
        }

        Ok(())
    }
}
