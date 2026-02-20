use std::collections::HashMap;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use distant_core::net::common::{ConnectionId, Host, Map, Request, Response};
use distant_core::net::manager::ManagerClient;
use distant_core::protocol::{
    self, semver, ChangeKind, ChangeKindSet, FileType, Permissions, SearchQuery,
    SearchQueryContentsMatch, SearchQueryMatch, SearchQueryPathMatch, SetPermissionsOptions,
    SystemInfo, Version,
};
use distant_core::{DistantChannel, DistantChannelExt, RemoteCommand, Searcher, Watcher};
use log::*;
use serde_json::json;
use tabled::settings::object::Rows;
use tabled::settings::style::Style;
use tabled::settings::{Alignment, Disable, Modify};
use tabled::{Table, Tabled};
use tokio::sync::mpsc;

use crate::cli::common::{
    connect_to_manager, try_connect as try_connect_no_autostart, Cache, JsonAuthHandler,
    MsgReceiver, MsgSender, PromptAuthHandler,
};
use crate::constants::MAX_PIPE_CHUNK_SIZE;
use crate::options::{
    ClientFileSystemSubcommand, ClientSubcommand, Format, ParseShellError, Shell as ShellOption,
};
use crate::{CliError, CliResult};

mod lsp;
mod shell;

use lsp::Lsp;
use shell::Shell;

use super::common::RemoteProcessLink;

const SLEEP_DURATION: Duration = Duration::from_millis(1);

pub fn run(cmd: ClientSubcommand) -> CliResult {
    let rt = tokio::runtime::Runtime::new().context("Failed to start up runtime")?;
    rt.block_on(async_run(cmd))
}

async fn read_cache(path: &Path) -> Cache {
    // If we get an error, just default anyway
    Cache::read_from_disk_or_default(path.to_path_buf())
        .await
        .unwrap_or_else(|_| Cache::new(path.to_path_buf()))
}

async fn async_run(cmd: ClientSubcommand) -> CliResult {
    match cmd {
        ClientSubcommand::Connect {
            cache,
            destination,
            format,
            network,
            options,
        } => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(format, network).await?;

            // Trigger our manager to connect to the launched server
            debug!("Connecting to server at {} with {}", destination, options);
            let id = match format {
                Format::Shell => client
                    .connect(*destination, options, PromptAuthHandler::new())
                    .await
                    .context("Failed to connect to server")?,
                Format::Json => client
                    .connect(*destination, options, JsonAuthHandler::default())
                    .await
                    .context("Failed to connect to server")?,
            };

            // Mark the server's id as the new default
            debug!("Updating selected connection id in cache to {}", id);
            let mut cache = read_cache(&cache).await;
            *cache.data.selected = id;
            cache.write_to_disk().await?;

            match format {
                Format::Shell => println!("{id}"),
                Format::Json => println!(
                    "{}",
                    serde_json::to_string(&json!({
                        "type": "connected",
                        "id": id,
                    }))
                    .unwrap()
                ),
            }
        }
        ClientSubcommand::Launch {
            cache,
            mut destination,
            distant_args,
            distant_bin,
            distant_bind_server,
            format,
            network,
            mut options,
        } => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(format, network).await?;

            // Grab the host we are connecting to for later use
            let host = destination.host.to_string();

            // If we have no scheme on launch, we need to fill it in with something
            //
            // TODO: Can we have the server support this instead of the client? Right now, the
            //       server is failing because it cannot parse //localhost/ as it fails with
            //       an invalid IPv4 or registered name character error on host
            if destination.scheme.is_none() {
                destination.scheme = Some("ssh".to_string());
            }

            // TODO: Handle this more cleanly
            if let Some(x) = distant_args {
                options.insert("distant.args".to_string(), x);
            }
            if let Some(x) = distant_bin {
                options.insert("distant.bin".to_string(), x);
            }
            if let Some(x) = distant_bind_server {
                options.insert("distant.bind_server".to_string(), x.to_string());
            }

            // Start the server using our manager
            debug!("Launching server at {} with {}", destination, options);
            let mut new_destination = match format {
                Format::Shell => client
                    .launch(*destination, options, PromptAuthHandler::new())
                    .await
                    .context("Failed to launch server")?,
                Format::Json => client
                    .launch(*destination, options, JsonAuthHandler::default())
                    .await
                    .context("Failed to launch server")?,
            };

            // Update the new destination with our previously-used host if the
            // new host is not globally-accessible
            if !new_destination.host.is_global() {
                trace!(
                    "Updating host to {:?} from non-global {:?}",
                    host,
                    new_destination.host.to_string()
                );
                new_destination.host = host
                    .parse::<Host>()
                    .map_err(|x| anyhow::anyhow!(x))
                    .context("Failed to replace host")?;
            } else {
                trace!("Host {:?} is global", new_destination.host.to_string());
            }

            // Trigger our manager to connect to the launched server
            debug!("Connecting to server at {}", new_destination);
            let id = match format {
                Format::Shell => client
                    .connect(new_destination, Map::new(), PromptAuthHandler::new())
                    .await
                    .context("Failed to connect to server")?,
                Format::Json => client
                    .connect(new_destination, Map::new(), JsonAuthHandler::default())
                    .await
                    .context("Failed to connect to server")?,
            };

            // Mark the server's id as the new default
            debug!("Updating selected connection id in cache to {}", id);
            let mut cache = read_cache(&cache).await;
            *cache.data.selected = id;
            cache.write_to_disk().await?;

            match format {
                Format::Shell => println!("{id}"),
                Format::Json => println!(
                    "{}",
                    serde_json::to_string(&json!({
                        "type": "launched",
                        "id": id,
                    }))
                    .unwrap()
                ),
            }
        }
        ClientSubcommand::Api {
            cache,
            connection,
            network,
            timeout,
        } => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(Format::Json, network).await?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            let timeout = match timeout {
                Some(timeout) if timeout.as_secs_f64() >= f64::EPSILON => Some(timeout),
                _ => None,
            };

            debug!("Opening raw channel to connection {}", connection_id);
            let mut channel = client
                .open_raw_channel(connection_id)
                .await
                .with_context(|| {
                    format!("Failed to open raw channel to connection {connection_id}")
                })?;

            debug!(
                "Timeout configured to be {}",
                match timeout {
                    Some(secs) => format!("{secs}s"),
                    None => "none".to_string(),
                }
            );

            debug!("Starting api tasks");
            let (msg_tx, mut msg_rx) = mpsc::channel(1);
            let request_task = tokio::spawn(async move {
                let mut rx = MsgReceiver::from_stdin()
                    .into_rx::<Request<protocol::Msg<protocol::Request>>>();
                loop {
                    match rx.recv().await {
                        Some(Ok(request)) => {
                            if let Err(x) = msg_tx.send(request).await {
                                error!("Failed to forward request: {x}");
                                break;
                            }
                        }
                        Some(Err(x)) => error!("{}", x),
                        None => {
                            debug!("Shutting down repl");
                            break;
                        }
                    }
                }
                io::Result::Ok(())
            });
            let channel_task = tokio::task::spawn(async move {
                let tx = MsgSender::from_stdout();

                loop {
                    let ready = channel.readable_or_writeable().await?;

                    // Keep track of whether we read or wrote anything
                    let mut read_blocked = !ready.is_readable();
                    let mut write_blocked = !ready.is_writable();

                    if ready.is_readable() {
                        match channel
                            .try_read_frame_as::<Response<protocol::Msg<protocol::Response>>>()
                        {
                            Ok(Some(msg)) => tx.send_blocking(&msg)?,
                            Ok(None) => break,
                            Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                                read_blocked = true;
                            }
                            Err(x) => return Err(x),
                        }
                    }

                    if ready.is_writable() {
                        if let Ok(msg) = msg_rx.try_recv() {
                            match channel.try_write_frame_for(&msg) {
                                Ok(_) => (),
                                Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                                    write_blocked = true
                                }
                                Err(x) => return Err(x),
                            }
                        } else {
                            match channel.try_flush() {
                                Ok(0) => write_blocked = true,
                                Ok(_) => (),
                                Err(x) if x.kind() == io::ErrorKind::WouldBlock => {
                                    write_blocked = true
                                }
                                Err(x) => {
                                    error!("Failed to flush outgoing data: {x}");
                                }
                            }
                        }
                    }

                    // If we did not read or write anything, sleep a bit to offload CPU usage
                    if read_blocked && write_blocked {
                        tokio::time::sleep(SLEEP_DURATION).await;
                    }
                }

                io::Result::Ok(())
            });

            let (r1, r2) = tokio::join!(request_task, channel_task);
            match r1 {
                Err(x) => error!("{}", x),
                Ok(Err(x)) => error!("{}", x),
                _ => (),
            }
            match r2 {
                Err(x) => error!("{}", x),
                Ok(Err(x)) => error!("{}", x),
                _ => (),
            }

            debug!("Shutting down repl");
        }
        ClientSubcommand::Shell {
            cache,
            cmd,
            connection,
            current_dir,
            environment,
            network,
        } => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(Format::Shell, network).await?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            debug!("Opening channel to connection {}", connection_id);
            let channel = client
                .open_raw_channel(connection_id)
                .await
                .with_context(|| format!("Failed to open channel to connection {connection_id}"))?;

            // Convert cmd into string
            let cmd = cmd.map(|cmd| cmd.join(" "));

            debug!(
                "Spawning shell (environment = {:?}): {}",
                environment,
                cmd.as_deref().unwrap_or(r"$SHELL")
            );
            Shell::new(channel.into_client().into_channel())
                .spawn(
                    cmd,
                    environment.into_map(),
                    current_dir,
                    MAX_PIPE_CHUNK_SIZE,
                )
                .await?;
        }
        ClientSubcommand::Spawn {
            cache,
            connection,
            cmd,
            cmd_str,
            current_dir,
            environment,
            lsp,
            pty,
            shell,
            network,
        } => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(Format::Shell, network).await?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            debug!("Opening channel to connection {}", connection_id);
            let mut channel: DistantChannel = client
                .open_raw_channel(connection_id)
                .await
                .with_context(|| format!("Failed to open channel to connection {connection_id}"))?
                .into_client()
                .into_channel();

            // Convert cmd into string
            let cmd = cmd_str.unwrap_or_else(|| cmd.join(" "));

            // Check if we should attempt to run the command in a shell
            let cmd = match shell {
                None => cmd,

                // Use default shell, which we need to figure out
                Some(None) => {
                    let system_info = channel
                        .system_info()
                        .await
                        .context("Failed to detect remote operating system")?;

                    // If system reports a default shell, use it, otherwise pick a default based on the
                    // operating system being windows or non-windows
                    let shell: ShellOption = if !system_info.shell.is_empty() {
                        system_info.shell.parse()
                    } else if system_info.family.eq_ignore_ascii_case("windows") {
                        "cmd.exe".parse()
                    } else {
                        "/bin/sh".parse()
                    }
                    .map_err(|x: ParseShellError| anyhow::anyhow!(x))?;

                    shell
                        .make_cmd_string(&cmd)
                        .map_err(|x| anyhow::anyhow!(x))?
                }

                // Use explicit shell
                Some(Some(shell)) => shell
                    .make_cmd_string(&cmd)
                    .map_err(|x| anyhow::anyhow!(x))?,
            };

            if let Some(scheme) = lsp {
                debug!(
                    "Spawning LSP server (pty = {}, cwd = {:?}): {}",
                    pty, current_dir, cmd
                );
                Lsp::new(channel)
                    .spawn(cmd, current_dir, scheme, pty, MAX_PIPE_CHUNK_SIZE)
                    .await?;
            } else if pty {
                debug!(
                    "Spawning pty process (environment = {:?}, cwd = {:?}): {}",
                    environment, current_dir, cmd
                );
                Shell::new(channel)
                    .spawn(
                        cmd,
                        environment.into_map(),
                        current_dir,
                        MAX_PIPE_CHUNK_SIZE,
                    )
                    .await?;
            } else {
                debug!(
                    "Spawning regular process (environment = {:?}, cwd = {:?}): {}",
                    environment, current_dir, cmd
                );
                let mut proc = RemoteCommand::new()
                    .environment(environment.into_map())
                    .current_dir(current_dir)
                    .pty(None)
                    .spawn(channel, &cmd)
                    .await
                    .with_context(|| format!("Failed to spawn {cmd}"))?;

                // Now, map the remote process' stdin/stdout/stderr to our own process
                let link = RemoteProcessLink::from_remote_pipes(
                    proc.stdin.take(),
                    proc.stdout.take().unwrap(),
                    proc.stderr.take().unwrap(),
                    MAX_PIPE_CHUNK_SIZE,
                );

                let status = proc.wait().await.context("Failed to wait for process")?;

                // Shut down our link
                link.shutdown().await;

                if !status.success {
                    if let Some(code) = status.code {
                        return Err(CliError::Exit(code as u8));
                    } else {
                        return Err(CliError::FAILURE);
                    }
                }
            }
        }
        ClientSubcommand::SystemInfo {
            cache,
            connection,
            network,
        } => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(Format::Shell, network).await?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            debug!("Opening channel to connection {}", connection_id);
            let channel = client
                .open_raw_channel(connection_id)
                .await
                .with_context(|| format!("Failed to open channel to connection {connection_id}"))?;

            debug!("Retrieving system information");
            let SystemInfo {
                family,
                os,
                arch,
                current_dir,
                main_separator,
                username,
                shell,
            } = channel
                .into_client()
                .into_channel()
                .system_info()
                .await
                .with_context(|| {
                    format!(
                        "Failed to retrieve system information using connection {connection_id}"
                    )
                })?;

            let mut out = std::io::stdout();

            out.write_all(
                &format!(
                    concat!(
                        "Family: {:?}\n",
                        "Operating System: {:?}\n",
                        "Arch: {:?}\n",
                        "Cwd: {:?}\n",
                        "Path Sep: {:?}\n",
                        "Username: {:?}\n",
                        "Shell: {:?}"
                    ),
                    family, os, arch, current_dir, main_separator, username, shell
                )
                .into_bytes(),
            )
            .context("Failed to write system information to stdout")?;
            out.flush().context("Failed to flush stdout")?;
        }
        ClientSubcommand::Version {
            cache,
            connection,
            format,
            network,
        } => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(format, network).await?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            debug!("Opening raw channel to connection {}", connection_id);
            let channel = client
                .open_raw_channel(connection_id)
                .await
                .with_context(|| {
                    format!("Failed to open raw channel to connection {connection_id}")
                })?;

            debug!("Retrieving version information");
            let version = channel
                .into_client()
                .into_channel()
                .version()
                .await
                .with_context(|| {
                    format!("Failed to retrieve version using connection {connection_id}")
                })?;

            match format {
                Format::Shell => {
                    let mut client_version: semver::Version = env!("CARGO_PKG_VERSION")
                        .parse()
                        .context("Failed to parse client version")?;

                    // Add the package name to the version information
                    if client_version.build.is_empty() {
                        client_version.build = semver::BuildMetadata::new(env!("CARGO_PKG_NAME"))
                            .context("Failed to define client build metadata")?;
                    } else {
                        let raw_build_str = format!(
                            "{}.{}",
                            client_version.build.as_str(),
                            env!("CARGO_PKG_NAME")
                        );
                        client_version.build = semver::BuildMetadata::new(&raw_build_str)
                            .context("Failed to define client build metadata")?;
                    }

                    println!(
                        "Client: {client_version} (Protocol {})",
                        distant_core::protocol::PROTOCOL_VERSION
                    );

                    println!(
                        "Server: {} (Protocol {})",
                        version.server_version, version.protocol_version
                    );

                    // Build a complete set of capabilities to show which ones we support
                    let mut capabilities: HashMap<String, u8> = Version::capabilities()
                        .iter()
                        .map(|cap| (cap.to_string(), 1))
                        .collect();

                    for cap in version.capabilities {
                        *capabilities.entry(cap).or_default() += 1;
                    }

                    let mut capabilities: Vec<String> = capabilities
                        .into_iter()
                        .map(|(cap, cnt)| {
                            if cnt > 1 {
                                format!("+{cap}")
                            } else {
                                format!("-{cap}")
                            }
                        })
                        .collect();
                    capabilities.sort_unstable();

                    // Figure out the text length of the longest capability
                    let max_len = capabilities.iter().map(|x| x.len()).max().unwrap_or(0);

                    if max_len > 0 {
                        const MAX_COLS: usize = 4;

                        // Determine how wide we have available to determine how many columns
                        // to use; if we don't have a terminal width, default to something
                        //
                        // Maximum columns we want to support is 4
                        let cols = match terminal_size::terminal_size() {
                            // If we have a tty, see how many we can fit including space char
                            //
                            // Ensure that we at least return 1 as cols
                            Some((width, _)) => std::cmp::max(width.0 as usize / (max_len + 1), 1),

                            // If we have no tty, default to 4 columns
                            None => MAX_COLS,
                        };

                        println!("Capabilities supported (+) or not (-):");
                        for chunk in capabilities.chunks(std::cmp::min(cols, MAX_COLS)) {
                            let cnt = chunk.len();
                            match cnt {
                                1 => println!("{:max_len$}", chunk[0]),
                                2 => println!("{:max_len$} {:max_len$}", chunk[0], chunk[1]),
                                3 => println!(
                                    "{:max_len$} {:max_len$} {:max_len$}",
                                    chunk[0], chunk[1], chunk[2]
                                ),
                                4 => println!(
                                    "{:max_len$} {:max_len$} {:max_len$} {:max_len$}",
                                    chunk[0], chunk[1], chunk[2], chunk[3]
                                ),
                                _ => unreachable!("Chunk of size {cnt} is not 1 > i <= {MAX_COLS}"),
                            }
                        }
                    }
                }
                Format::Json => {
                    println!("{}", serde_json::to_string(&version).unwrap())
                }
            }
        }
        ClientSubcommand::FileSystem(ClientFileSystemSubcommand::Copy {
            cache,
            connection,
            network,
            src,
            dst,
        }) => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(Format::Shell, network).await?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            debug!("Opening channel to connection {}", connection_id);
            let channel = client
                .open_raw_channel(connection_id)
                .await
                .with_context(|| format!("Failed to open channel to connection {connection_id}"))?;

            debug!("Copying {src:?} to {dst:?}");
            channel
                .into_client()
                .into_channel()
                .copy(src.as_path(), dst.as_path())
                .await
                .with_context(|| {
                    format!("Failed to copy {src:?} to {dst:?} using connection {connection_id}")
                })?;
        }
        ClientSubcommand::FileSystem(ClientFileSystemSubcommand::Exists {
            cache,
            connection,
            network,
            path,
        }) => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(Format::Shell, network).await?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            debug!("Opening channel to connection {}", connection_id);
            let channel = client
                .open_raw_channel(connection_id)
                .await
                .with_context(|| format!("Failed to open channel to connection {connection_id}"))?;

            debug!("Checking existence of {path:?}");
            let exists = channel
                .into_client()
                .into_channel()
                .exists(path.as_path())
                .await
                .with_context(|| {
                    format!(
                        "Failed to check existence of {path:?} using connection {connection_id}"
                    )
                })?;

            if exists {
                println!("true");
            } else {
                println!("false");
            }
        }
        ClientSubcommand::FileSystem(ClientFileSystemSubcommand::MakeDir {
            cache,
            connection,
            network,
            path,
            all,
        }) => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(Format::Shell, network).await?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            debug!("Opening channel to connection {}", connection_id);
            let channel = client
                .open_raw_channel(connection_id)
                .await
                .with_context(|| format!("Failed to open channel to connection {connection_id}"))?;

            debug!("Making directory {path:?} (all = {all})");
            channel
                .into_client()
                .into_channel()
                .create_dir(path.as_path(), all)
                .await
                .with_context(|| {
                    format!("Failed to make directory {path:?} using connection {connection_id}")
                })?;
        }
        ClientSubcommand::FileSystem(ClientFileSystemSubcommand::Metadata {
            cache,
            connection,
            network,
            canonicalize,
            resolve_file_type,
            path,
        }) => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(Format::Shell, network).await?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            debug!("Opening channel to connection {}", connection_id);
            let channel = client
                .open_raw_channel(connection_id)
                .await
                .with_context(|| format!("Failed to open channel to connection {connection_id}"))?;

            debug!("Retrieving metadata of {path:?}");
            let metadata = channel
                .into_client()
                .into_channel()
                .metadata(path.as_path(), canonicalize, resolve_file_type)
                .await
                .with_context(|| {
                    format!(
                        "Failed to retrieve metadata of {path:?} using connection {connection_id}"
                    )
                })?;

            println!(
                concat!(
                    "{}",
                    "Type: {}\n",
                    "Len: {}\n",
                    "Readonly: {}\n",
                    "Created: {}\n",
                    "Last Accessed: {}\n",
                    "Last Modified: {}\n",
                    "{}",
                    "{}",
                    "{}",
                ),
                metadata
                    .canonicalized_path
                    .map(|p| format!("Canonicalized Path: {p:?}\n"))
                    .unwrap_or_default(),
                metadata.file_type.as_ref(),
                metadata.len,
                metadata.readonly,
                metadata.created.unwrap_or_default(),
                metadata.accessed.unwrap_or_default(),
                metadata.modified.unwrap_or_default(),
                metadata
                    .unix
                    .map(|u| format!(
                        concat!(
                            "Owner Read: {}\n",
                            "Owner Write: {}\n",
                            "Owner Exec: {}\n",
                            "Group Read: {}\n",
                            "Group Write: {}\n",
                            "Group Exec: {}\n",
                            "Other Read: {}\n",
                            "Other Write: {}\n",
                            "Other Exec: {}",
                        ),
                        u.owner_read,
                        u.owner_write,
                        u.owner_exec,
                        u.group_read,
                        u.group_write,
                        u.group_exec,
                        u.other_read,
                        u.other_write,
                        u.other_exec
                    ))
                    .unwrap_or_default(),
                metadata
                    .windows
                    .map(|w| format!(
                        concat!(
                            "Archive: {}\n",
                            "Compressed: {}\n",
                            "Encrypted: {}\n",
                            "Hidden: {}\n",
                            "Integrity Stream: {}\n",
                            "Normal: {}\n",
                            "Not Content Indexed: {}\n",
                            "No Scrub Data: {}\n",
                            "Offline: {}\n",
                            "Recall on Data Access: {}\n",
                            "Recall on Open: {}\n",
                            "Reparse Point: {}\n",
                            "Sparse File: {}\n",
                            "System: {}\n",
                            "Temporary: {}",
                        ),
                        w.archive,
                        w.compressed,
                        w.encrypted,
                        w.hidden,
                        w.integrity_stream,
                        w.normal,
                        w.not_content_indexed,
                        w.no_scrub_data,
                        w.offline,
                        w.recall_on_data_access,
                        w.recall_on_open,
                        w.reparse_point,
                        w.sparse_file,
                        w.system,
                        w.temporary,
                    ))
                    .unwrap_or_default(),
                if metadata.unix.is_none() && metadata.windows.is_none() {
                    String::from("\n")
                } else {
                    String::new()
                }
            )
        }
        ClientSubcommand::FileSystem(ClientFileSystemSubcommand::Read {
            cache,
            connection,
            network,
            path,
            depth,
            absolute,
            canonicalize,
            include_root,
        }) => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(Format::Shell, network).await?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            debug!("Opening channel to connection {}", connection_id);
            let mut channel: DistantChannel = client
                .open_raw_channel(connection_id)
                .await
                .with_context(|| format!("Failed to open channel to connection {connection_id}"))?
                .into_client()
                .into_channel();

            // NOTE: We don't know whether the path is for a file or directory, so we try both
            //       at the same time and return the first result, or fail if both fail!
            debug!(
                "Reading {path:?} (depth = {}, absolute = {}, canonicalize = {}, include_root = {})",
                depth, absolute, canonicalize, include_root
            );
            let results = channel
                .send(protocol::Msg::Batch(vec![
                    protocol::Request::FileRead {
                        path: path.to_path_buf(),
                    },
                    protocol::Request::DirRead {
                        path: path.to_path_buf(),
                        depth,
                        absolute,
                        canonicalize,
                        include_root,
                    },
                ]))
                .await
                .with_context(|| {
                    format!("Failed to read {path:?} using connection {connection_id}")
                })?;

            let mut errors = Vec::new();
            for response in results
                .payload
                .into_batch()
                .context("Got single response to batch request")?
            {
                match response {
                    protocol::Response::DirEntries { entries, .. } => {
                        #[derive(Tabled)]
                        struct EntryRow {
                            ty: String,
                            path: String,
                        }

                        let data = Table::new(entries.into_iter().map(|entry| EntryRow {
                            ty: String::from(match entry.file_type {
                                FileType::Dir => "<DIR>",
                                FileType::File => "",
                                FileType::Symlink => "<SYMLINK>",
                            }),
                            path: entry.path.to_string_lossy().to_string(),
                        }))
                        .with(Style::blank())
                        .with(Disable::row(Rows::new(..1)))
                        .with(Modify::new(Rows::new(..)).with(Alignment::left()))
                        .to_string()
                        .into_bytes();

                        let mut out = std::io::stdout();
                        out.write_all(&data)
                            .context("Failed to write directory contents to stdout")?;
                        out.flush().context("Failed to flush stdout")?;
                        return Ok(());
                    }
                    protocol::Response::Blob { data } => {
                        let mut out = std::io::stdout();
                        out.write_all(&data)
                            .context("Failed to write file contents to stdout")?;
                        out.flush().context("Failed to flush stdout")?;
                        return Ok(());
                    }
                    protocol::Response::Error(x) => errors.push(x),
                    _ => continue,
                }
            }

            if let Some(x) = errors.first() {
                return Err(CliError::from(anyhow::anyhow!(x.to_io_error())));
            }
        }
        ClientSubcommand::FileSystem(ClientFileSystemSubcommand::Remove {
            cache,
            connection,
            network,
            path,
            force,
        }) => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(Format::Shell, network).await?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            debug!("Opening channel to connection {}", connection_id);
            let channel = client
                .open_raw_channel(connection_id)
                .await
                .with_context(|| format!("Failed to open channel to connection {connection_id}"))?;

            debug!("Removing {path:?} (force = {force}");
            channel
                .into_client()
                .into_channel()
                .remove(path.as_path(), force)
                .await
                .with_context(|| {
                    format!("Failed to remove {path:?} using connection {connection_id}")
                })?;
        }
        ClientSubcommand::FileSystem(ClientFileSystemSubcommand::Rename {
            cache,
            connection,
            network,
            src,
            dst,
        }) => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(Format::Shell, network).await?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            debug!("Opening channel to connection {}", connection_id);
            let channel = client
                .open_raw_channel(connection_id)
                .await
                .with_context(|| format!("Failed to open channel to connection {connection_id}"))?;

            debug!("Renaming {src:?} to {dst:?}");
            channel
                .into_client()
                .into_channel()
                .rename(src.as_path(), dst.as_path())
                .await
                .with_context(|| {
                    format!("Failed to rename {src:?} to {dst:?} using connection {connection_id}")
                })?;
        }
        ClientSubcommand::FileSystem(ClientFileSystemSubcommand::Search {
            cache,
            connection,
            network,
            target,
            condition,
            options,
            paths,
        }) => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(Format::Shell, network).await?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            debug!("Opening channel to connection {}", connection_id);
            let channel = client
                .open_raw_channel(connection_id)
                .await
                .with_context(|| format!("Failed to open channel to connection {connection_id}"))?;

            let query = SearchQuery {
                target: target.into(),
                condition,
                paths,
                options: options.into(),
            };

            let mut searcher = Searcher::search(channel.into_client().into_channel(), query)
                .await
                .context("Failed to start search")?;

            // Continue to receive and process matches
            let mut last_searched_path: Option<PathBuf> = None;
            while let Some(m) = searcher.next().await {
                let mut files: HashMap<_, Vec<String>> = HashMap::new();
                let mut is_targeting_paths = false;

                match m {
                    SearchQueryMatch::Path(SearchQueryPathMatch { path, .. }) => {
                        // Create the entry with no lines called out
                        files.entry(path).or_default();
                        is_targeting_paths = true;
                    }

                    SearchQueryMatch::Contents(SearchQueryContentsMatch {
                        path,
                        lines,
                        line_number,
                        ..
                    }) => {
                        let file_matches = files.entry(path).or_default();

                        file_matches.push(format!(
                            "{line_number}:{}",
                            lines.to_string_lossy().trim_end()
                        ));
                    }
                }

                let mut output = String::new();
                for (path, lines) in files {
                    use std::fmt::Write;

                    // If we are seeing a new path, print it out
                    if last_searched_path.as_deref() != Some(path.as_path()) {
                        // If we have already seen some path before, we would have printed it, and
                        // we want to add a space between it and the current path, but only if we are
                        // printing out file content matches and not paths
                        if last_searched_path.is_some() && !is_targeting_paths {
                            writeln!(&mut output).unwrap();
                        }

                        writeln!(&mut output, "{}", path.to_string_lossy()).unwrap();
                    }

                    for line in lines {
                        writeln!(&mut output, "{line}").unwrap();
                    }

                    // Update our last seen path
                    last_searched_path = Some(path);
                }

                if !output.is_empty() {
                    print!("{}", output);
                }
            }
        }
        ClientSubcommand::FileSystem(ClientFileSystemSubcommand::SetPermissions {
            cache,
            connection,
            network,
            follow_symlinks,
            recursive,
            mode,
            path,
        }) => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(Format::Shell, network).await?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            debug!("Opening channel to connection {}", connection_id);
            let mut channel: DistantChannel = client
                .open_raw_channel(connection_id)
                .await
                .with_context(|| format!("Failed to open channel to connection {connection_id}"))?
                .into_client()
                .into_channel();

            debug!("Parsing {mode:?} into a proper set of permissions");
            let permissions = {
                if mode.trim().eq_ignore_ascii_case("readonly") {
                    Permissions::readonly()
                } else if mode.trim().eq_ignore_ascii_case("notreadonly") {
                    Permissions::writable()
                } else {
                    // Attempt to parse an octal number (chmod absolute), falling back to
                    // parsing the mode string similar to chmod's symbolic mode
                    match u32::from_str_radix(&mode, 8) {
                        Ok(absolute) => {
                            Permissions::from_unix_mode(file_mode::Mode::from(absolute).mode())
                        }
                        Err(_) => {
                            // The way parsing works, we need to parse and apply to two different
                            // situations
                            //
                            // 1. A mode that is all 1s so we can see if the mask would remove
                            //    permission to some of the bits
                            // 2. A mode that is all 0s so we can see if the mask would add
                            //    permission to some of the bits
                            let mut removals = file_mode::Mode::from(0o777);
                            removals
                                .set_str(&mode)
                                .context("Failed to parse mode string")?;
                            let removals_mask = !removals.mode();

                            let mut additions = file_mode::Mode::empty();
                            additions
                                .set_str(&mode)
                                .context("Failed to parse mode string")?;
                            let additions_mask = additions.mode();

                            macro_rules! get_mode {
                                ($mask:expr) => {{
                                    let is_false = removals_mask & $mask > 0;
                                    let is_true = additions_mask & $mask > 0;
                                    match (is_true, is_false) {
                                        (true, false) => Some(true),
                                        (false, true) => Some(false),
                                        (false, false) => None,
                                        (true, true) => {
                                            unreachable!("Mask cannot be adding and removing")
                                        }
                                    }
                                }};
                            }

                            Permissions {
                                owner_read: get_mode!(0o400),
                                owner_write: get_mode!(0o200),
                                owner_exec: get_mode!(0o100),
                                group_read: get_mode!(0o040),
                                group_write: get_mode!(0o020),
                                group_exec: get_mode!(0o010),
                                other_read: get_mode!(0o004),
                                other_write: get_mode!(0o002),
                                other_exec: get_mode!(0o001),
                            }
                        }
                    }
                }
            };

            let options = SetPermissionsOptions {
                recursive,
                follow_symlinks,
                exclude_symlinks: false,
            };
            debug!("Setting permissions for {path:?} as (permissions = {permissions:?}, options = {options:?})");
            channel
                .set_permissions(path.as_path(), permissions, options)
                .await
                .with_context(|| {
                    format!(
                        "Failed to set permissions for {path:?} using connection {connection_id}"
                    )
                })?;
        }
        ClientSubcommand::FileSystem(ClientFileSystemSubcommand::Watch {
            cache,
            connection,
            network,
            recursive,
            only,
            except,
            path,
        }) => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(Format::Shell, network).await?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            debug!("Opening channel to connection {}", connection_id);
            let channel = client
                .open_raw_channel(connection_id)
                .await
                .with_context(|| format!("Failed to open channel to connection {connection_id}"))?;

            debug!("Special request creating watcher for {:?}", path);
            let mut watcher = Watcher::watch(
                channel.into_client().into_channel(),
                path.as_path(),
                recursive,
                only.into_iter().collect::<ChangeKindSet>(),
                except.into_iter().collect::<ChangeKindSet>(),
            )
            .await
            .with_context(|| format!("Failed to watch {path:?}"))?;

            // Continue to receive and process changes
            while let Some(change) = watcher.next().await {
                println!(
                    "{} {}",
                    match change.kind {
                        ChangeKind::Create => "(Created)",
                        ChangeKind::Delete => "(Removed)",
                        x if x.is_access() => "(Accessed)",
                        x if x.is_modify() => "(Modified)",
                        x if x.is_rename() => "(Renamed)",
                        _ => "(Affected)",
                    },
                    change.path.to_string_lossy()
                );
            }
        }
        ClientSubcommand::FileSystem(ClientFileSystemSubcommand::Write {
            cache,
            connection,
            network,
            append,
            path,
            data,
        }) => {
            let data = match data {
                Some(x) => match x.into_string() {
                    Ok(x) => x.into_bytes(),
                    Err(_) => {
                        return Err(CliError::from(anyhow::anyhow!(
                            "Non-unicode input is disallowed!"
                        )));
                    }
                },
                None => {
                    debug!("No data provided, reading from stdin");
                    use std::io::Read;
                    let mut buf = Vec::new();
                    std::io::stdin()
                        .read_to_end(&mut buf)
                        .context("Failed to read stdin")?;
                    buf
                }
            };

            debug!("Connecting to manager");
            let mut client = connect_to_manager(Format::Shell, network).await?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            debug!("Opening channel to connection {}", connection_id);
            let channel = client
                .open_raw_channel(connection_id)
                .await
                .with_context(|| format!("Failed to open channel to connection {connection_id}"))?;

            if append {
                debug!("Appending contents to {path:?}");
                channel
                    .into_client()
                    .into_channel()
                    .append_file(path.as_path(), data)
                    .await
                    .with_context(|| {
                        format!("Failed to write to {path:?} using connection {connection_id}")
                    })?;
            } else {
                debug!("Writing contents to {path:?}");
                channel
                    .into_client()
                    .into_channel()
                    .write_file(path.as_path(), data)
                    .await
                    .with_context(|| {
                        format!("Failed to write to {path:?} using connection {connection_id}")
                    })?;
            }
        }
        ClientSubcommand::Ssh {
            cache,
            destination,
            options,
            network,
            current_dir,
            environment,
            cmd,
        } => {
            debug!("Connecting to manager (auto-start enabled)");
            let mut client = connect_to_manager(Format::Shell, network).await?;

            // Ensure destination has ssh:// scheme
            let mut destination = *destination;
            if destination.scheme.is_none() {
                destination.scheme = Some("ssh".to_string());
            }

            // Connect via SSH (pure SSH mode — no distant binary needed on remote)
            debug!("Connecting via SSH to {}", destination);
            let dest_display = destination.to_string();
            let id = client
                .connect(destination, options, PromptAuthHandler::new())
                .await
                .with_context(|| format!("Failed to connect to {dest_display}"))?;

            // Update cache with the new connection
            let mut cache = read_cache(&cache).await;
            *cache.data.selected = id;
            cache.write_to_disk().await?;

            debug!("Opening channel to connection {}", id);
            let channel = client
                .open_raw_channel(id)
                .await
                .with_context(|| format!("Failed to open channel to connection {id}"))?;

            // Convert cmd into string
            let cmd = cmd.map(|cmd| cmd.join(" "));

            debug!(
                "Spawning shell (environment = {:?}): {}",
                environment,
                cmd.as_deref().unwrap_or(r"$SHELL")
            );
            Shell::new(channel.into_client().into_channel())
                .spawn(
                    cmd,
                    environment.into_map(),
                    current_dir,
                    MAX_PIPE_CHUNK_SIZE,
                )
                .await?;
        }
        ClientSubcommand::Status { network, cache } => {
            // Try to connect to the manager without auto-starting it
            match try_connect_no_autostart(Format::Shell, &network).await {
                Ok(mut client) => {
                    println!("Manager: running");

                    let list = client
                        .list()
                        .await
                        .context("Failed to get list of connections")?;

                    let selected = read_cache(&cache).await.data.selected;

                    if list.is_empty() {
                        println!("\nNo active connections.");
                    } else {
                        println!("\nConnections:");
                        for (id, dest) in list {
                            let marker = if *selected == id { " *" } else { "  " };
                            let scheme = dest
                                .scheme
                                .as_ref()
                                .map(|s| format!("{s}://"))
                                .unwrap_or_default();
                            let port = dest.port.map(|p| format!(":{p}")).unwrap_or_default();
                            println!("{marker} {id} -> {scheme}{}{port}", dest.host);
                        }
                    }
                }
                Err(_) => {
                    println!("Manager: not running");
                    println!(
                        "\nStart it with:\n  \
                         distant manager listen --daemon"
                    );
                }
            }
        }
    }

    Ok(())
}

async fn use_or_lookup_connection_id(
    cache: &mut Cache,
    connection: Option<ConnectionId>,
    client: &mut ManagerClient,
) -> anyhow::Result<ConnectionId> {
    match connection {
        Some(id) => {
            trace!("Using specified connection id: {}", id);
            Ok(id)
        }
        None => {
            trace!("Looking up connection id");
            let list = client
                .list()
                .await
                .context("Failed to retrieve list of available connections")?;

            if list.contains_key(&cache.data.selected) {
                trace!("Using cached connection id: {}", cache.data.selected);
                Ok(*cache.data.selected)
            } else if list.is_empty() {
                trace!("Cached connection id is invalid as there are no connections");
                anyhow::bail!(
                    "No active connections.\n\n\
                     Connect to a remote host first:\n  \
                     distant connect ssh://user@host\n  \
                     distant launch ssh://user@host  (requires distant installed on remote)\n\n\
                     Or use the shorthand:\n  \
                     distant ssh user@host"
                );
            } else if list.len() > 1 {
                trace!("Cached connection id is invalid and there are multiple connections");
                anyhow::bail!(
                    "Multiple active connections. Specify which one to use:\n  \
                     distant manager list              (see available connections)\n  \
                     distant manager select            (choose interactively)\n  \
                     distant shell --connection ID     (specify directly)"
                );
            } else {
                trace!("Cached connection id is invalid");
                *cache.data.selected = *list.keys().next().unwrap();
                trace!(
                    "Detected singular connection id, so updating cache: {}",
                    cache.data.selected
                );
                cache.write_to_disk().await?;
                Ok(*cache.data.selected)
            }
        }
    }
}
