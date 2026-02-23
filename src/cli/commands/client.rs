use std::collections::HashMap;
use std::io;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use console::style;
use distant_core::net::common::{ConnectionId, Destination, Host, Map, Request, Response};
use distant_core::net::manager::ManagerClient;
use distant_core::protocol::{
    self, semver, ChangeKind, ChangeKindSet, FileType, Permissions, SearchQuery,
    SearchQueryContentsMatch, SearchQueryMatch, SearchQueryPathMatch, SetPermissionsOptions,
    SystemInfo, Version,
};
use distant_core::{Channel, ChannelExt, RemoteCommand, Searcher, Watcher};
use log::*;
use serde_json::json;
use tabled::settings::object::Rows;
use tabled::settings::style::Style;
use tabled::settings::{Alignment, Disable, Modify};
use tabled::{Table, Tabled};
use tokio::sync::mpsc;

use dialoguer::console::Term;
use dialoguer::theme::ColorfulTheme;
use dialoguer::Select;

use crate::cli::common::{
    connect_to_manager, format_connection, try_connect as try_connect_no_autostart, Cache,
    JsonAuthHandler, MsgReceiver, MsgSender, PromptAuthHandler, Ui,
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
    let ui = Ui::new();

    match cmd {
        ClientSubcommand::Connect {
            cache,
            destination,
            format,
            network,
            options,
            new,
        } => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(format, network, &ui).await?;

            // Check for an existing connection to the same destination
            let (id, reused) = if !new {
                if let Some(existing_id) =
                    find_existing_connection_id(&mut client, &destination).await
                {
                    let dest_display = destination.to_string();
                    match format {
                        Format::Shell => {
                            ui.success(&format!(
                                "Reusing existing connection to {dest_display} (id: {existing_id})"
                            ));
                        }
                        Format::Json => {}
                    }
                    (existing_id, true)
                } else {
                    // No existing connection found, create a new one
                    debug!("Connecting to server at {} with {}", destination, options);
                    let dest_display = destination.to_string();
                    let sp = ui.spinner(&format!("Connecting to {dest_display}..."));
                    let id = match format {
                        Format::Shell => {
                            client
                                .connect(
                                    *destination,
                                    options,
                                    PromptAuthHandler::with_progress_bar(sp.progress_bar()),
                                )
                                .await
                        }
                        Format::Json => {
                            client
                                .connect(*destination, options, JsonAuthHandler::default())
                                .await
                        }
                    };
                    match &id {
                        Ok(id) => sp.done(&format!("Connected (id: {id})")),
                        Err(_) => sp.fail(&format!("Connection to {dest_display} failed")),
                    }
                    (id.context("Failed to connect to server")?, false)
                }
            } else {
                // --new flag: always create a fresh connection
                debug!("Connecting to server at {} with {}", destination, options);
                let dest_display = destination.to_string();
                let sp = ui.spinner(&format!("Connecting to {dest_display}..."));
                let id = match format {
                    Format::Shell => {
                        client
                            .connect(
                                *destination,
                                options,
                                PromptAuthHandler::with_progress_bar(sp.progress_bar()),
                            )
                            .await
                    }
                    Format::Json => {
                        client
                            .connect(*destination, options, JsonAuthHandler::default())
                            .await
                    }
                };
                match &id {
                    Ok(id) => sp.done(&format!("Connected (id: {id})")),
                    Err(_) => sp.fail(&format!("Connection to {dest_display} failed")),
                }
                (id.context("Failed to connect to server")?, false)
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
                        "reused": reused,
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
            let mut client = connect_to_manager(format, network, &ui).await?;

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
            let sp = ui.spinner(&format!("Launching server at {}...", destination));
            let launch_result = match format {
                Format::Shell => {
                    client
                        .launch(
                            *destination,
                            options,
                            PromptAuthHandler::with_progress_bar(sp.progress_bar()),
                        )
                        .await
                }
                Format::Json => {
                    client
                        .launch(*destination, options, JsonAuthHandler::default())
                        .await
                }
            };
            match &launch_result {
                Ok(_) => sp.done(&format!("Server launched at {host}")),
                Err(_) => sp.fail("Failed to launch server"),
            }
            let mut new_destination = launch_result.context("Failed to launch server")?;

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
            let sp = ui.spinner("Connecting to launched server...");
            let id = match format {
                Format::Shell => {
                    client
                        .connect(
                            new_destination,
                            Map::new(),
                            PromptAuthHandler::with_progress_bar(sp.progress_bar()),
                        )
                        .await
                }
                Format::Json => {
                    client
                        .connect(new_destination, Map::new(), JsonAuthHandler::default())
                        .await
                }
            };
            match &id {
                Ok(id) => sp.done(&format!("Connected (id: {id})")),
                Err(_) => sp.fail("Connection to launched server failed"),
            }
            let id = id.context("Failed to connect to server")?;

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
            let mut client = try_connect_no_autostart(Format::Json, &network)
                .await
                .context(
                    "Failed to connect to the distant manager. \
                     Is it running? Start it with: distant manager listen --daemon",
                )?;

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
            let mut client = connect_to_manager(Format::Shell, network, &ui).await?;

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
            let mut client = connect_to_manager(Format::Shell, network, &ui).await?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            debug!("Opening channel to connection {}", connection_id);
            let mut channel: Channel = client
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
            let mut client = connect_to_manager(Format::Shell, network, &ui).await?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            debug!("Opening channel to connection {}", connection_id);
            let channel = client
                .open_raw_channel(connection_id)
                .await
                .with_context(|| format!("Failed to open channel to connection {connection_id}"))?;

            debug!("Retrieving system information");
            let info = channel
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
            out.write_all(&format_system_info(&info).into_bytes())
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
            let mut client = connect_to_manager(format, network, &ui).await?;

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
                    print!("{}", format_version_shell(&version)?);
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
            let mut client = connect_to_manager(Format::Shell, network, &ui).await?;

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
            let mut client = connect_to_manager(Format::Shell, network, &ui).await?;

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
            let mut client = connect_to_manager(Format::Shell, network, &ui).await?;

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
            let mut client = connect_to_manager(Format::Shell, network, &ui).await?;

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

            print!("{}", format_metadata(&metadata))
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
            let mut client = connect_to_manager(Format::Shell, network, &ui).await?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            debug!("Opening channel to connection {}", connection_id);
            let mut channel: Channel = client
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

            let data = process_read_response(results.payload)?;
            let mut out = std::io::stdout();
            out.write_all(&data)
                .context("Failed to write contents to stdout")?;
            out.flush().context("Failed to flush stdout")?;
            if !data.is_empty() {
                return Ok(());
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
            let mut client = connect_to_manager(Format::Shell, network, &ui).await?;

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
            let mut client = connect_to_manager(Format::Shell, network, &ui).await?;

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
            let mut client = connect_to_manager(Format::Shell, network, &ui).await?;

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
                let (output, path, _is_targeting_paths) =
                    format_search_match(m, &last_searched_path);

                if !output.is_empty() {
                    print!("{}", output);
                }

                last_searched_path = Some(path);
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
            let mut client = connect_to_manager(Format::Shell, network, &ui).await?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            debug!("Opening channel to connection {}", connection_id);
            let mut channel: Channel = client
                .open_raw_channel(connection_id)
                .await
                .with_context(|| format!("Failed to open channel to connection {connection_id}"))?
                .into_client()
                .into_channel();

            debug!("Parsing {mode:?} into a proper set of permissions");
            let permissions = parse_permissions_mode(&mode)?;

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
            let mut client = connect_to_manager(Format::Shell, network, &ui).await?;

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

            ui.dim(&format!("Watching {:?}", path));

            // Continue to receive and process changes
            while let Some(change) = watcher.next().await {
                println!(
                    "{} {}",
                    format_change_kind(change.kind),
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
            let mut client = connect_to_manager(Format::Shell, network, &ui).await?;

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
            new,
            cmd,
        } => {
            debug!("Connecting to manager (auto-start enabled)");
            let mut client = connect_to_manager(Format::Shell, network, &ui).await?;

            // Ensure destination has ssh:// scheme
            let mut destination = *destination;
            if destination.scheme.is_none() {
                destination.scheme = Some("ssh".to_string());
            }

            // Check for an existing connection or create a new one
            let id = if !new {
                if let Some(existing_id) =
                    find_existing_connection_id(&mut client, &destination).await
                {
                    let dest_display = destination.to_string();
                    ui.success(&format!(
                        "Reusing existing connection to {dest_display} (id: {existing_id})"
                    ));
                    existing_id
                } else {
                    // No existing connection found, create a new one
                    debug!("Connecting via SSH to {}", destination);
                    let dest_display = destination.to_string();
                    let sp = ui.spinner(&format!("Connecting to {}...", dest_display));
                    let result = client
                        .connect(
                            destination,
                            options,
                            PromptAuthHandler::with_progress_bar(sp.progress_bar()),
                        )
                        .await;
                    match &result {
                        Ok(_) => sp.done(&format!("Connected to {dest_display}")),
                        Err(_) => sp.fail(&format!("Connection to {dest_display} failed")),
                    }
                    result.with_context(|| format!("Failed to connect to {dest_display}"))?
                }
            } else {
                // --new flag: always create a fresh connection
                debug!("Connecting via SSH to {}", destination);
                let dest_display = destination.to_string();
                let sp = ui.spinner(&format!("Connecting to {}...", dest_display));
                let result = client
                    .connect(
                        destination,
                        options,
                        PromptAuthHandler::with_progress_bar(sp.progress_bar()),
                    )
                    .await;
                match &result {
                    Ok(_) => sp.done(&format!("Connected to {dest_display}")),
                    Err(_) => sp.fail(&format!("Connection to {dest_display} failed")),
                }
                result.with_context(|| format!("Failed to connect to {dest_display}"))?
            };

            // Update cache with the connection
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
        ClientSubcommand::Status {
            id,
            format,
            network,
            cache,
        } => {
            match id {
                Some(id) => {
                    // Detail mode: show info about a specific connection
                    debug!("Connecting to manager");
                    let mut client = connect_to_manager(Format::Shell, network, &ui).await?;

                    debug!("Getting info about connection {}", id);
                    let info = client
                        .info(id)
                        .await
                        .context("Failed to get info about connection")?;
                    debug!("Got info: {info:?}");

                    match format {
                        Format::Json => {
                            println!(
                                "{}",
                                serde_json::to_string(&info)
                                    .context("Failed to format connection info as json")?
                            );
                        }
                        Format::Shell => {
                            let (header, host_str, options_str) =
                                format_connection_detail(info.id, &info.destination, &info.options);

                            ui.header(&header);
                            ui.write_line(&format!("  {}  {}", style("Host:").bold(), host_str));

                            if let Some(opts) = options_str {
                                ui.write_line(&format!("  {}  {}", style("Options:").bold(), opts));
                            }
                        }
                    }
                }
                None => {
                    // Overview mode: show manager status + connection list
                    match try_connect_no_autostart(Format::Shell, &network).await {
                        Ok(mut client) => {
                            let list = client
                                .list()
                                .await
                                .context("Failed to get list of connections")?;

                            let selected = read_cache(&cache).await.data.selected;

                            match format {
                                Format::Json => {
                                    println!(
                                        "{}",
                                        serde_json::to_string(&list)
                                            .context("Failed to format connection list as json")?
                                    );
                                }
                                Format::Shell => {
                                    ui.status(
                                        "Manager",
                                        "running",
                                        crate::cli::common::StatusColor::Green,
                                    );

                                    if list.is_empty() {
                                        ui.dim("\nNo active connections.");
                                    } else {
                                        ui.header("\nConnections:");
                                        for (id, dest) in list {
                                            let scheme = dest
                                                .scheme
                                                .as_ref()
                                                .map(|s| format!("{s}://"))
                                                .unwrap_or_default();
                                            let port = dest
                                                .port
                                                .map(|p| format!(":{p}"))
                                                .unwrap_or_default();
                                            if *selected == id {
                                                ui.write_line(&format!(
                                                    "  {} {} -> {scheme}{}{port}",
                                                    style("*").green(),
                                                    style(id).bold(),
                                                    dest.host
                                                ));
                                            } else {
                                                ui.write_line(&format!(
                                                    "    {} -> {scheme}{}{port}",
                                                    style(id).dim(),
                                                    dest.host
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Err(_) => match format {
                            Format::Shell => {
                                ui.status(
                                    "Manager",
                                    "not running",
                                    crate::cli::common::StatusColor::Red,
                                );
                                ui.dim("\n  Start it with: distant manager listen --daemon");
                            }
                            Format::Json => {
                                println!(
                                    "{}",
                                    serde_json::to_string(&serde_json::json!({})).unwrap()
                                );
                            }
                        },
                    }
                }
            }
        }
        ClientSubcommand::Kill {
            format,
            id,
            network,
            cache,
        } => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(format, network, &ui).await?;

            // Fetch list BEFORE kill for destination info + selection prompt
            let list = client
                .list()
                .await
                .context("Failed to get list of connections")?;

            let id = match id {
                Some(id) => id,
                None => {
                    if list.is_empty() {
                        return Err(CliError::Error(anyhow::anyhow!(
                            "No active connections.\n\n\
                             Connect to a remote host first:\n  \
                             distant connect ssh://user@host\n  \
                             distant ssh user@host"
                        )));
                    }

                    match format {
                        Format::Shell => {
                            if !Term::stderr().is_term() {
                                return Err(CliError::Error(anyhow::anyhow!(
                                    "No connection ID specified. See available connections:\n  \
                                     distant status"
                                )));
                            }

                            // Always show prompt — even with 1 connection — so user can cancel
                            let items: Vec<String> = list
                                .iter()
                                .map(|(id, dest)| format_connection(*id, dest))
                                .collect();
                            let selection = Select::with_theme(&ColorfulTheme::default())
                                .with_prompt("Select connection to kill")
                                .items(&items)
                                .default(0)
                                .interact_on_opt(&Term::stderr())
                                .context("Failed to render prompt")?;
                            match selection {
                                Some(index) => *list.keys().nth(index).unwrap(),
                                None => return Ok(()),
                            }
                        }
                        Format::Json => {
                            return Err(CliError::Error(anyhow::anyhow!(
                                "Connection ID is required in JSON mode"
                            )));
                        }
                    }
                }
            };

            debug!("Killing connection {}", id);
            client
                .kill(id)
                .await
                .with_context(|| format!("Failed to kill connection to server {id}"))?;

            debug!("Connection killed");
            match format {
                Format::Json => println!("{}", json!({"type": "ok", "id": id})),
                Format::Shell => {
                    let msg = match list.get(&id) {
                        Some(dest) => format!("Killed {}", format_connection(id, dest)),
                        None => format!("Killed connection {id}"),
                    };
                    ui.success(&msg);
                }
            }

            // Cache update — only if we killed the selected connection
            let mut cache = Cache::read_from_disk_or_default(cache)
                .await
                .context("Failed to read cache")?;
            if *cache.data.selected == id {
                let remaining = client
                    .list()
                    .await
                    .context("Failed to get updated connection list")?;
                if remaining.len() == 1 {
                    let new_id = *remaining.keys().next().unwrap();
                    *cache.data.selected = new_id;
                    if let Format::Shell = format {
                        if let Some(dest) = remaining.get(&new_id) {
                            ui.dim(&format!(
                                "Selected remaining connection: {}",
                                format_connection(new_id, dest)
                            ));
                        }
                    }
                } else {
                    *cache.data.selected = 0;
                }
                cache.write_to_disk().await?;
            }
        }
        ClientSubcommand::Select {
            format,
            connection,
            network,
            cache,
        } => {
            let mut cache = Cache::read_from_disk_or_default(cache)
                .await
                .context("Failed to look up cache")?;

            match connection {
                Some(id) => {
                    *cache.data.selected = id;
                    cache.write_to_disk().await?;
                }
                None => {
                    debug!("Connecting to manager");
                    let mut client = connect_to_manager(format, network, &ui).await?;
                    let list = client
                        .list()
                        .await
                        .context("Failed to get a list of managed connections")?;

                    if list.is_empty() {
                        return Err(CliError::Error(anyhow::anyhow!(
                            "No active connections.\n\n\
                             Connect to a remote host first:\n  \
                             distant connect ssh://user@host\n  \
                             distant ssh user@host"
                        )));
                    }

                    // Figure out the current selection
                    let current = list
                        .iter()
                        .enumerate()
                        .find_map(|(i, (id, _))| {
                            if *cache.data.selected == *id {
                                Some(i)
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default();

                    trace!("Building selection prompt of {} choices", list.len());
                    let items: Vec<String> = list
                        .iter()
                        .map(|(id, dest)| format_connection(*id, dest))
                        .collect();

                    // Prompt for a selection, with None meaning no change
                    let selected = match format {
                        Format::Shell => {
                            trace!("Rendering prompt");
                            Select::with_theme(&ColorfulTheme::default())
                                .items(&items)
                                .default(current)
                                .interact_on_opt(&Term::stderr())
                                .context("Failed to render prompt")?
                        }

                        Format::Json => {
                            // Print out choices
                            MsgSender::from_stdout()
                                .send_blocking(&json!({
                                    "type": "select",
                                    "choices": items,
                                    "current": current,
                                }))
                                .context("Failed to send JSON choices")?;

                            // Wait for a response
                            let msg = MsgReceiver::from_stdin()
                                .recv_blocking::<serde_json::Value>()
                                .context("Failed to receive JSON selection")?;

                            // Verify the response type is "selected"
                            match msg.get("type") {
                                Some(value) if value == "selected" => msg
                                    .get("choice")
                                    .and_then(|value| value.as_u64())
                                    .map(|choice| choice as usize),
                                Some(value) => {
                                    return Err(CliError::Error(anyhow::anyhow!(
                                        "Unexpected 'type' field value: {value}"
                                    )));
                                }
                                None => {
                                    return Err(CliError::Error(anyhow::anyhow!(
                                        "Missing 'type' field"
                                    )));
                                }
                            }
                        }
                    };

                    match selected {
                        Some(index) => {
                            trace!("Selected choice {}", index);
                            if let Some((id, _)) = list.iter().nth(index) {
                                debug!("Updating selected connection id in cache to {}", id);
                                *cache.data.selected = *id;
                                cache.write_to_disk().await?;
                            }
                        }
                        None => {
                            debug!("No change in selection of default connection id");
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Format a `ChangeKind` as a human-readable label for watch output.
fn format_change_kind(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Create => "(Created)",
        ChangeKind::Delete => "(Removed)",
        x if x.is_access() => "(Accessed)",
        x if x.is_modify() => "(Modified)",
        x if x.is_rename() => "(Renamed)",
        _ => "(Affected)",
    }
}

/// Format a `SystemInfo` struct into a human-readable multi-line string.
fn format_system_info(info: &SystemInfo) -> String {
    format!(
        concat!(
            "Family: {:?}\n",
            "Operating System: {:?}\n",
            "Arch: {:?}\n",
            "Cwd: {:?}\n",
            "Path Sep: {:?}\n",
            "Username: {:?}\n",
            "Shell: {:?}"
        ),
        info.family,
        info.os,
        info.arch,
        info.current_dir,
        info.main_separator,
        info.username,
        info.shell
    )
}

/// Format connection detail for `status <id>` output.
///
/// Returns `(header, host_line, options_line)` where `options_line` is `None`
/// when the options map is empty. The caller is responsible for terminal
/// styling (e.g. bold labels via `console::style`).
fn format_connection_detail(
    id: ConnectionId,
    dest: &Destination,
    options: &Map,
) -> (String, String, Option<String>) {
    let scheme = dest.scheme.as_deref().unwrap_or_default();
    let user = dest
        .username
        .as_deref()
        .map(|u| format!("{u}@"))
        .unwrap_or_default();
    let host = &dest.host;
    let port = dest.port.map(|p| format!(":{p}")).unwrap_or_default();

    let header = format!("Connection {id}:");
    let host_line = format!("{scheme}://{user}{host}{port}");
    let opts = options.to_string();
    let options_line = if opts.is_empty() { None } else { Some(opts) };

    (header, host_line, options_line)
}

/// Parses a permission mode string into a [`Permissions`] value.
///
/// Supports three formats:
/// - `"readonly"` / `"notreadonly"` — named permission shortcuts
/// - Octal string (e.g. `"755"`) — absolute chmod-style mode
/// - Symbolic string (e.g. `"u+x"`, `"go-w"`) — chmod symbolic mode
fn parse_permissions_mode(mode: &str) -> anyhow::Result<Permissions> {
    if mode.trim().eq_ignore_ascii_case("readonly") {
        return Ok(Permissions::readonly());
    }
    if mode.trim().eq_ignore_ascii_case("notreadonly") {
        return Ok(Permissions::writable());
    }

    // Attempt to parse an octal number (chmod absolute), falling back to
    // parsing the mode string similar to chmod's symbolic mode
    match u32::from_str_radix(mode, 8) {
        Ok(absolute) => Ok(Permissions::from_unix_mode(
            file_mode::Mode::from(absolute).mode(),
        )),
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
                .set_str(mode)
                .context("Failed to parse mode string")?;
            let removals_mask = !removals.mode();

            let mut additions = file_mode::Mode::empty();
            additions
                .set_str(mode)
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

            Ok(Permissions {
                owner_read: get_mode!(0o400),
                owner_write: get_mode!(0o200),
                owner_exec: get_mode!(0o100),
                group_read: get_mode!(0o040),
                group_write: get_mode!(0o020),
                group_exec: get_mode!(0o010),
                other_read: get_mode!(0o004),
                other_write: get_mode!(0o002),
                other_exec: get_mode!(0o001),
            })
        }
    }
}

/// Formats metadata for shell output, returning the formatted string.
fn format_metadata(metadata: &protocol::Metadata) -> String {
    format!(
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
            .as_ref()
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
            .as_ref()
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
            .as_ref()
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

/// Processes a batch read response (file read + dir read), returning the output bytes.
///
/// Returns `Ok(bytes)` with the file contents or directory table on success.
/// Returns `Err` if both the file and directory reads fail.
fn process_read_response(results: protocol::Msg<protocol::Response>) -> anyhow::Result<Vec<u8>> {
    let mut errors = Vec::new();
    for response in results
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

                return Ok(data);
            }
            protocol::Response::Blob { data } => {
                return Ok(data);
            }
            protocol::Response::Error(x) => errors.push(x),
            _ => continue,
        }
    }

    if let Some(x) = errors.first() {
        return Err(anyhow::anyhow!(x.to_io_error()));
    }

    Ok(Vec::new())
}

/// Formats a single search match into output text.
///
/// Returns the formatted output string and the path of the match (for tracking last-seen path).
fn format_search_match(
    m: SearchQueryMatch,
    last_searched_path: &Option<PathBuf>,
) -> (String, PathBuf, bool) {
    let (path, line, is_targeting_paths) = match m {
        SearchQueryMatch::Path(SearchQueryPathMatch { path, .. }) => (path, None, true),
        SearchQueryMatch::Contents(SearchQueryContentsMatch {
            path,
            lines,
            line_number,
            ..
        }) => {
            let line = format!("{line_number}:{}", lines.to_string_lossy().trim_end());
            (path, Some(line), false)
        }
    };

    let mut output = String::new();
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

    if let Some(line) = line {
        writeln!(&mut output, "{line}").unwrap();
    }

    (output, path, is_targeting_paths)
}

/// Formats version information for shell output.
fn format_version_shell(version: &Version) -> anyhow::Result<String> {
    let mut output = String::new();
    use std::fmt::Write;

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

    writeln!(
        &mut output,
        "Client: {client_version} (Protocol {})",
        distant_core::protocol::PROTOCOL_VERSION
    )
    .unwrap();

    writeln!(
        &mut output,
        "Server: {} (Protocol {})",
        version.server_version, version.protocol_version
    )
    .unwrap();

    // Build a complete set of capabilities to show which ones we support
    let mut capabilities: HashMap<String, u8> = Version::capabilities()
        .iter()
        .map(|cap| (cap.to_string(), 1))
        .collect();

    for cap in &version.capabilities {
        *capabilities.entry(cap.clone()).or_default() += 1;
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

        writeln!(&mut output, "Capabilities supported (+) or not (-):").unwrap();
        for chunk in capabilities.chunks(std::cmp::min(cols, MAX_COLS)) {
            let line: String = chunk
                .iter()
                .map(|c| format!("{c:max_len$}"))
                .collect::<Vec<_>>()
                .join(" ");
            writeln!(&mut output, "{line}").unwrap();
        }
    }

    Ok(output)
}

/// Checks for an existing connection matching the given destination's (scheme, host, port, username).
/// Returns the first matching connection ID, or None if no match found.
async fn find_existing_connection_id(
    client: &mut ManagerClient,
    dest: &Destination,
) -> Option<ConnectionId> {
    let list = match client.list().await {
        Ok(list) => list,
        Err(err) => {
            debug!("Failed to list connections for reuse check: {err}");
            return None;
        }
    };

    list.iter()
        .find(|(_, existing)| {
            let scheme_matches = match (&existing.scheme, &dest.scheme) {
                (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
                (None, None) => true,
                _ => false,
            };
            scheme_matches
                && existing.host == dest.host
                && existing.port == dest.port
                && existing.username == dest.username
        })
        .map(|(id, _)| *id)
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

                // If stderr is a TTY, let user pick interactively
                if console::Term::stderr().is_term() {
                    let items: Vec<String> = list
                        .iter()
                        .map(|(id, dest)| {
                            let scheme = dest
                                .scheme
                                .as_ref()
                                .map(|s| format!("{s}://"))
                                .unwrap_or_default();
                            let user = dest
                                .username
                                .as_ref()
                                .map(|u| format!("{u}@"))
                                .unwrap_or_default();
                            let port = dest.port.map(|p| format!(":{p}")).unwrap_or_default();
                            format!("{id} -> {scheme}{user}{}{port}", dest.host)
                        })
                        .collect();
                    let selection = dialoguer::Select::new()
                        .with_prompt("Multiple connections available")
                        .items(&items)
                        .default(0)
                        .interact_on_opt(&console::Term::stderr())
                        .context("Failed to select connection")?;
                    let Some(selection) = selection else {
                        anyhow::bail!("Cancelled");
                    };
                    let id = *list.keys().nth(selection).unwrap();
                    *cache.data.selected = id;
                    cache.write_to_disk().await?;
                    return Ok(id);
                }

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

#[cfg(test)]
mod tests {
    use super::*;
    use distant_core::protocol::{
        self, DirEntry, FileType, Metadata, Msg, Permissions, SearchQueryContentsMatch,
        SearchQueryMatch, SearchQueryPathMatch, UnixMetadata, Version, WindowsMetadata,
    };
    use std::path::PathBuf;

    // =====================================================================
    // parse_permissions_mode
    // =====================================================================

    mod parse_permissions_mode_tests {
        use super::*;

        #[test]
        fn readonly_keyword() {
            let p = parse_permissions_mode("readonly").unwrap();
            assert_eq!(p, Permissions::readonly());
        }

        #[test]
        fn readonly_keyword_case_insensitive() {
            let p = parse_permissions_mode("READONLY").unwrap();
            assert_eq!(p, Permissions::readonly());
        }

        #[test]
        fn readonly_keyword_with_whitespace() {
            let p = parse_permissions_mode("  readonly  ").unwrap();
            assert_eq!(p, Permissions::readonly());
        }

        #[test]
        fn notreadonly_keyword() {
            let p = parse_permissions_mode("notreadonly").unwrap();
            assert_eq!(p, Permissions::writable());
        }

        #[test]
        fn notreadonly_keyword_case_insensitive() {
            let p = parse_permissions_mode("NotReadOnly").unwrap();
            assert_eq!(p, Permissions::writable());
        }

        #[test]
        fn octal_755() {
            let p = parse_permissions_mode("755").unwrap();
            assert_eq!(p.owner_read, Some(true));
            assert_eq!(p.owner_write, Some(true));
            assert_eq!(p.owner_exec, Some(true));
            assert_eq!(p.group_read, Some(true));
            assert_eq!(p.group_write, Some(false));
            assert_eq!(p.group_exec, Some(true));
            assert_eq!(p.other_read, Some(true));
            assert_eq!(p.other_write, Some(false));
            assert_eq!(p.other_exec, Some(true));
        }

        #[test]
        fn octal_644() {
            let p = parse_permissions_mode("644").unwrap();
            assert_eq!(p.owner_read, Some(true));
            assert_eq!(p.owner_write, Some(true));
            assert_eq!(p.owner_exec, Some(false));
            assert_eq!(p.group_read, Some(true));
            assert_eq!(p.group_write, Some(false));
            assert_eq!(p.group_exec, Some(false));
            assert_eq!(p.other_read, Some(true));
            assert_eq!(p.other_write, Some(false));
            assert_eq!(p.other_exec, Some(false));
        }

        #[test]
        fn octal_000() {
            let p = parse_permissions_mode("000").unwrap();
            assert_eq!(p.owner_read, Some(false));
            assert_eq!(p.owner_write, Some(false));
            assert_eq!(p.owner_exec, Some(false));
            assert_eq!(p.group_read, Some(false));
            assert_eq!(p.group_write, Some(false));
            assert_eq!(p.group_exec, Some(false));
            assert_eq!(p.other_read, Some(false));
            assert_eq!(p.other_write, Some(false));
            assert_eq!(p.other_exec, Some(false));
        }

        #[test]
        fn octal_777() {
            let p = parse_permissions_mode("777").unwrap();
            assert_eq!(p.owner_read, Some(true));
            assert_eq!(p.owner_write, Some(true));
            assert_eq!(p.owner_exec, Some(true));
            assert_eq!(p.group_read, Some(true));
            assert_eq!(p.group_write, Some(true));
            assert_eq!(p.group_exec, Some(true));
            assert_eq!(p.other_read, Some(true));
            assert_eq!(p.other_write, Some(true));
            assert_eq!(p.other_exec, Some(true));
        }

        #[test]
        fn symbolic_u_plus_x() {
            let p = parse_permissions_mode("u+x").unwrap();
            assert_eq!(p.owner_exec, Some(true));
            // Other bits should be None (unchanged)
            assert_eq!(p.owner_read, None);
            assert_eq!(p.owner_write, None);
        }

        #[test]
        fn symbolic_go_minus_w() {
            let p = parse_permissions_mode("go-w").unwrap();
            assert_eq!(p.group_write, Some(false));
            assert_eq!(p.other_write, Some(false));
            // Owner write should be unchanged
            assert_eq!(p.owner_write, None);
        }

        #[test]
        fn symbolic_a_plus_r() {
            let p = parse_permissions_mode("a+r").unwrap();
            assert_eq!(p.owner_read, Some(true));
            assert_eq!(p.group_read, Some(true));
            assert_eq!(p.other_read, Some(true));
        }

        #[test]
        fn symbolic_u_minus_rwx() {
            let p = parse_permissions_mode("u-rwx").unwrap();
            assert_eq!(p.owner_read, Some(false));
            assert_eq!(p.owner_write, Some(false));
            assert_eq!(p.owner_exec, Some(false));
        }

        #[test]
        fn invalid_mode_string_returns_error() {
            assert!(parse_permissions_mode("zzz-invalid").is_err());
        }
    }

    // =====================================================================
    // format_metadata
    // =====================================================================

    mod format_metadata_tests {
        use super::*;

        fn minimal_metadata() -> Metadata {
            Metadata {
                canonicalized_path: None,
                file_type: FileType::File,
                len: 1024,
                readonly: false,
                accessed: None,
                created: None,
                modified: None,
                unix: None,
                windows: None,
            }
        }

        #[test]
        fn minimal_metadata_output() {
            let output = format_metadata(&minimal_metadata());
            assert!(output.contains("Type: file"));
            assert!(output.contains("Len: 1024"));
            assert!(output.contains("Readonly: false"));
            // Should not contain platform-specific fields
            assert!(!output.contains("Owner Read"));
            assert!(!output.contains("Archive"));
        }

        #[test]
        fn metadata_with_canonicalized_path() {
            let mut m = minimal_metadata();
            m.canonicalized_path = Some(PathBuf::from("/resolved/path"));
            let output = format_metadata(&m);
            assert!(output.contains("Canonicalized Path:"));
            assert!(output.contains("/resolved/path"));
        }

        #[test]
        fn metadata_with_timestamps() {
            let mut m = minimal_metadata();
            m.created = Some(1000);
            m.accessed = Some(2000);
            m.modified = Some(3000);
            let output = format_metadata(&m);
            assert!(output.contains("Created: 1000"));
            assert!(output.contains("Last Accessed: 2000"));
            assert!(output.contains("Last Modified: 3000"));
        }

        #[test]
        fn metadata_with_unix_permissions() {
            let mut m = minimal_metadata();
            m.unix = Some(UnixMetadata {
                owner_read: true,
                owner_write: true,
                owner_exec: false,
                group_read: true,
                group_write: false,
                group_exec: false,
                other_read: true,
                other_write: false,
                other_exec: false,
            });
            let output = format_metadata(&m);
            assert!(output.contains("Owner Read: true"));
            assert!(output.contains("Owner Write: true"));
            assert!(output.contains("Owner Exec: false"));
            assert!(output.contains("Group Read: true"));
            assert!(output.contains("Other Read: true"));
        }

        #[test]
        fn metadata_dir_type() {
            let mut m = minimal_metadata();
            m.file_type = FileType::Dir;
            let output = format_metadata(&m);
            assert!(output.contains("Type: dir"));
        }

        #[test]
        fn metadata_symlink_type() {
            let mut m = minimal_metadata();
            m.file_type = FileType::Symlink;
            let output = format_metadata(&m);
            assert!(output.contains("Type: symlink"));
        }

        #[test]
        fn metadata_with_windows_attributes() {
            let mut m = minimal_metadata();
            m.windows = Some(WindowsMetadata {
                archive: true,
                compressed: false,
                encrypted: false,
                hidden: true,
                integrity_stream: false,
                normal: false,
                not_content_indexed: false,
                no_scrub_data: false,
                offline: false,
                recall_on_data_access: false,
                recall_on_open: false,
                reparse_point: false,
                sparse_file: false,
                system: true,
                temporary: false,
            });
            let output = format_metadata(&m);
            assert!(output.contains("Archive: true"), "output: {output}");
            assert!(output.contains("Hidden: true"), "output: {output}");
            assert!(output.contains("System: true"), "output: {output}");
            assert!(output.contains("Compressed: false"), "output: {output}");
            assert!(output.contains("Temporary: false"), "output: {output}");
            // Should not contain unix fields
            assert!(!output.contains("Owner Read"), "output: {output}");
        }
    }

    // =====================================================================
    // process_read_response
    // =====================================================================

    mod process_read_response_tests {
        use super::*;

        #[test]
        fn returns_file_contents_from_blob() {
            let response = Msg::Batch(vec![
                protocol::Response::Error(protocol::Error {
                    kind: protocol::ErrorKind::NotFound,
                    description: "not a file".into(),
                }),
                protocol::Response::Blob {
                    data: b"hello world".to_vec(),
                },
            ]);
            let data = process_read_response(response).unwrap();
            assert_eq!(data, b"hello world");
        }

        #[test]
        fn returns_directory_table_from_dir_entries() {
            let response = Msg::Batch(vec![
                protocol::Response::Error(protocol::Error {
                    kind: protocol::ErrorKind::NotFound,
                    description: "not a file".into(),
                }),
                protocol::Response::DirEntries {
                    entries: vec![
                        DirEntry {
                            path: PathBuf::from("subdir"),
                            file_type: FileType::Dir,
                            depth: 1,
                        },
                        DirEntry {
                            path: PathBuf::from("file.txt"),
                            file_type: FileType::File,
                            depth: 1,
                        },
                    ],
                    errors: vec![],
                },
            ]);
            let data = process_read_response(response).unwrap();
            let output = String::from_utf8(data).unwrap();
            assert!(output.contains("<DIR>"));
            assert!(output.contains("subdir"));
            assert!(output.contains("file.txt"));
        }

        #[test]
        fn returns_error_when_all_responses_are_errors() {
            let response = Msg::Batch(vec![
                protocol::Response::Error(protocol::Error {
                    kind: protocol::ErrorKind::NotFound,
                    description: "file not found".into(),
                }),
                protocol::Response::Error(protocol::Error {
                    kind: protocol::ErrorKind::NotFound,
                    description: "dir not found".into(),
                }),
            ]);
            assert!(process_read_response(response).is_err());
        }

        #[test]
        fn returns_error_for_single_response() {
            let response = Msg::Single(protocol::Response::Blob {
                data: b"data".to_vec(),
            });
            // Should fail because we expect a batch
            assert!(process_read_response(response).is_err());
        }

        #[test]
        fn prefers_first_successful_response() {
            // If both blob and dir_entries are present, the first one wins
            let response = Msg::Batch(vec![
                protocol::Response::Blob {
                    data: b"file data".to_vec(),
                },
                protocol::Response::DirEntries {
                    entries: vec![],
                    errors: vec![],
                },
            ]);
            let data = process_read_response(response).unwrap();
            assert_eq!(data, b"file data");
        }

        #[test]
        fn symlink_entry_shows_symlink_tag() {
            let response = Msg::Batch(vec![protocol::Response::DirEntries {
                entries: vec![DirEntry {
                    path: PathBuf::from("link"),
                    file_type: FileType::Symlink,
                    depth: 1,
                }],
                errors: vec![],
            }]);
            let data = process_read_response(response).unwrap();
            let output = String::from_utf8(data).unwrap();
            assert!(output.contains("<SYMLINK>"));
        }
    }

    // =====================================================================
    // format_search_match
    // =====================================================================

    mod format_search_match_tests {
        use super::*;

        #[test]
        fn path_match_prints_path() {
            let m = SearchQueryMatch::Path(SearchQueryPathMatch {
                path: PathBuf::from("/foo/bar"),
                submatches: vec![],
            });
            let (output, path, is_path) = format_search_match(m, &None);
            assert!(output.contains("/foo/bar"));
            assert_eq!(path, PathBuf::from("/foo/bar"));
            assert!(is_path);
        }

        #[test]
        fn contents_match_prints_line_number_and_text() {
            let m = SearchQueryMatch::Contents(SearchQueryContentsMatch {
                path: PathBuf::from("/foo/bar.rs"),
                lines: protocol::SearchQueryMatchData::Text("fn main() {}".into()),
                line_number: 42,
                absolute_offset: 100,
                submatches: vec![],
            });
            let (output, path, is_path) = format_search_match(m, &None);
            assert!(output.contains("/foo/bar.rs"));
            assert!(output.contains("42:fn main() {}"));
            assert_eq!(path, PathBuf::from("/foo/bar.rs"));
            assert!(!is_path);
        }

        #[test]
        fn same_path_as_last_does_not_reprint_path() {
            let last = Some(PathBuf::from("/foo/bar.rs"));
            let m = SearchQueryMatch::Contents(SearchQueryContentsMatch {
                path: PathBuf::from("/foo/bar.rs"),
                lines: protocol::SearchQueryMatchData::Text("line 2".into()),
                line_number: 10,
                absolute_offset: 50,
                submatches: vec![],
            });
            let (output, _path, _is_path) = format_search_match(m, &last);
            // Should contain the line but NOT re-print the path
            assert!(output.contains("10:line 2"));
            // Count occurrences of the path - should only appear in the line match, not as a header
            assert!(!output.starts_with("/foo/bar.rs\n"));
        }

        #[test]
        fn new_path_with_previous_adds_blank_line_for_contents() {
            let last = Some(PathBuf::from("/old/path.rs"));
            let m = SearchQueryMatch::Contents(SearchQueryContentsMatch {
                path: PathBuf::from("/new/path.rs"),
                lines: protocol::SearchQueryMatchData::Text("content".into()),
                line_number: 1,
                absolute_offset: 0,
                submatches: vec![],
            });
            let (output, _path, _is_path) = format_search_match(m, &last);
            // Should start with a blank line separator, then the new path
            assert!(output.starts_with('\n'));
            assert!(output.contains("/new/path.rs"));
        }
    }

    // =====================================================================
    // format_version_shell
    // =====================================================================

    mod format_version_shell_tests {
        use super::*;

        fn make_version(caps: Vec<&str>) -> Version {
            Version {
                server_version: "0.20.0".parse().unwrap(),
                protocol_version: "0.1.0".parse().unwrap(),
                capabilities: caps.into_iter().map(String::from).collect(),
            }
        }

        #[test]
        fn includes_client_version_line() {
            let version = make_version(vec![]);
            let output = format_version_shell(&version).unwrap();
            assert!(output.contains("Client:"));
            assert!(output.contains("Protocol"));
        }

        #[test]
        fn includes_server_version_line() {
            let version = make_version(vec![]);
            let output = format_version_shell(&version).unwrap();
            assert!(output.contains("Server: 0.20.0"));
            assert!(output.contains("Protocol 0.1.0"));
        }

        #[test]
        fn shows_matching_capabilities_as_supported() {
            // Server reports capabilities that the client also supports
            let client_caps: Vec<String> = Version::capabilities()
                .iter()
                .map(|s| s.to_string())
                .collect();
            let version = make_version(client_caps.iter().map(|s| s.as_str()).collect());
            let output = format_version_shell(&version).unwrap();
            // All capabilities should show as supported (+)
            assert!(output.contains("Capabilities supported (+) or not (-):"));
            for cap in &client_caps {
                assert!(
                    output.contains(&format!("+{cap}")),
                    "Expected +{cap} in output"
                );
            }
        }

        #[test]
        fn shows_missing_capabilities_as_unsupported() {
            // Server reports NO capabilities
            let version = make_version(vec![]);
            let output = format_version_shell(&version).unwrap();
            // Client capabilities should show as unsupported (-)
            let client_caps = Version::capabilities();
            if !client_caps.is_empty() {
                assert!(output.contains("Capabilities supported (+) or not (-):"));
                for cap in client_caps {
                    assert!(
                        output.contains(&format!("-{cap}")),
                        "Expected -{cap} in output"
                    );
                }
            }
        }

        #[test]
        fn empty_capabilities_skips_capabilities_section() {
            // If BOTH client and server have no capabilities, skip the section
            // (In practice the client always has some, so this tests the "max_len == 0" path)
            // We can't easily test this since Version::capabilities() is fixed,
            // but we test that the function at least doesn't panic.
            let version = make_version(vec![]);
            let _output = format_version_shell(&version).unwrap();
        }
    }

    // =====================================================================
    // format_change_kind
    // =====================================================================

    mod format_change_kind_tests {
        use super::*;

        #[test]
        fn create_returns_created() {
            assert_eq!(format_change_kind(ChangeKind::Create), "(Created)");
        }

        #[test]
        fn delete_returns_removed() {
            assert_eq!(format_change_kind(ChangeKind::Delete), "(Removed)");
        }

        #[test]
        fn access_variants_return_accessed() {
            assert_eq!(format_change_kind(ChangeKind::Access), "(Accessed)");
            assert_eq!(format_change_kind(ChangeKind::Open), "(Accessed)");
            assert_eq!(format_change_kind(ChangeKind::CloseWrite), "(Accessed)");
            assert_eq!(format_change_kind(ChangeKind::CloseNoWrite), "(Accessed)");
        }

        #[test]
        fn modify_variants_return_modified() {
            assert_eq!(format_change_kind(ChangeKind::Modify), "(Modified)");
            assert_eq!(format_change_kind(ChangeKind::Attribute), "(Modified)");
        }

        #[test]
        fn rename_returns_renamed() {
            assert_eq!(format_change_kind(ChangeKind::Rename), "(Renamed)");
        }

        #[test]
        fn unknown_returns_affected() {
            assert_eq!(format_change_kind(ChangeKind::Unknown), "(Affected)");
        }
    }

    // =====================================================================
    // format_system_info
    // =====================================================================

    mod format_system_info_tests {
        use super::*;

        fn make_system_info() -> SystemInfo {
            SystemInfo {
                family: "unix".to_string(),
                os: "linux".to_string(),
                arch: "x86_64".to_string(),
                current_dir: PathBuf::from("/home/user"),
                main_separator: '/',
                username: "testuser".to_string(),
                shell: "bash".to_string(),
            }
        }

        #[test]
        fn includes_family() {
            let output = format_system_info(&make_system_info());
            assert!(output.contains("Family: \"unix\""), "output: {output}");
        }

        #[test]
        fn includes_os() {
            let output = format_system_info(&make_system_info());
            assert!(
                output.contains("Operating System: \"linux\""),
                "output: {output}"
            );
        }

        #[test]
        fn includes_arch() {
            let output = format_system_info(&make_system_info());
            assert!(output.contains("Arch: \"x86_64\""), "output: {output}");
        }

        #[test]
        fn includes_current_dir() {
            let output = format_system_info(&make_system_info());
            assert!(output.contains("Cwd: \"/home/user\""), "output: {output}");
        }

        #[test]
        fn includes_separator() {
            let output = format_system_info(&make_system_info());
            assert!(output.contains("Path Sep: '/'"), "output: {output}");
        }

        #[test]
        fn includes_username() {
            let output = format_system_info(&make_system_info());
            assert!(
                output.contains("Username: \"testuser\""),
                "output: {output}"
            );
        }

        #[test]
        fn includes_shell() {
            let output = format_system_info(&make_system_info());
            assert!(output.contains("Shell: \"bash\""), "output: {output}");
        }
    }

    // =====================================================================
    // format_connection_detail
    // =====================================================================

    mod format_connection_detail_tests {
        use super::*;

        #[test]
        fn includes_connection_id_and_host() {
            let dest = Destination {
                scheme: Some("ssh".to_string()),
                username: Some("user".to_string()),
                password: None,
                host: Host::Name("example.com".to_string()),
                port: Some(22),
            };
            let (header, host, _) = format_connection_detail(42, &dest, &Map::new());
            assert_eq!(header, "Connection 42:");
            assert_eq!(host, "ssh://user@example.com:22");
        }

        #[test]
        fn includes_options_when_present() {
            let dest = Destination {
                scheme: Some("distant".to_string()),
                username: None,
                password: None,
                host: Host::Name("server.local".to_string()),
                port: None,
            };
            let mut opts = Map::new();
            opts.insert("key".to_string(), "value".to_string());
            let (_, _, options) = format_connection_detail(1, &dest, &opts);
            assert!(options.is_some(), "expected options to be present");
        }

        #[test]
        fn no_options_line_when_empty() {
            let dest = Destination {
                scheme: Some("ssh".to_string()),
                username: None,
                password: None,
                host: Host::Name("host".to_string()),
                port: None,
            };
            let (_, _, options) = format_connection_detail(1, &dest, &Map::new());
            assert!(options.is_none(), "expected no options");
        }

        #[test]
        fn handles_missing_optional_fields() {
            let dest = Destination {
                scheme: None,
                username: None,
                password: None,
                host: Host::Name("localhost".to_string()),
                port: None,
            };
            let (_, host, _) = format_connection_detail(1, &dest, &Map::new());
            assert_eq!(host, "://localhost");
        }
    }

    // =====================================================================
    // find_existing_connection_id (uses mock ManagerClient)
    // =====================================================================

    mod find_existing_connection_id_tests {
        use super::*;
        use distant_core::net::common::{FramedTransport, InmemoryTransport, Request, Response};
        use distant_core::net::manager::{ConnectionList, ManagerRequest, ManagerResponse};

        fn setup() -> (ManagerClient, FramedTransport<InmemoryTransport>) {
            let (t1, t2) = FramedTransport::pair(100);
            let client = ManagerClient::spawn_inmemory(t2, Default::default());
            (client, t1)
        }

        #[tokio::test]
        async fn returns_none_when_list_is_empty() {
            let (mut client, mut transport) = setup();

            tokio::spawn(async move {
                let req: Request<ManagerRequest> =
                    transport.read_frame_as().await.unwrap().unwrap();
                transport
                    .write_frame_for(&Response::new(
                        req.id,
                        ManagerResponse::List(ConnectionList::new()),
                    ))
                    .await
                    .unwrap();
            });

            let dest: Destination = "ssh://user@host".parse().unwrap();
            let result = find_existing_connection_id(&mut client, &dest).await;
            assert_eq!(result, None);
        }

        #[tokio::test]
        async fn returns_matching_connection_id() {
            let (mut client, mut transport) = setup();

            tokio::spawn(async move {
                let req: Request<ManagerRequest> =
                    transport.read_frame_as().await.unwrap().unwrap();
                let mut list = ConnectionList::new();
                list.insert(42, "ssh://user@host".parse::<Destination>().unwrap());
                list.insert(99, "ssh://other@elsewhere".parse::<Destination>().unwrap());
                transport
                    .write_frame_for(&Response::new(req.id, ManagerResponse::List(list)))
                    .await
                    .unwrap();
            });

            let dest: Destination = "ssh://user@host".parse().unwrap();
            let result = find_existing_connection_id(&mut client, &dest).await;
            assert_eq!(result, Some(42));
        }

        #[tokio::test]
        async fn returns_none_when_no_match() {
            let (mut client, mut transport) = setup();

            tokio::spawn(async move {
                let req: Request<ManagerRequest> =
                    transport.read_frame_as().await.unwrap().unwrap();
                let mut list = ConnectionList::new();
                list.insert(99, "ssh://other@elsewhere".parse::<Destination>().unwrap());
                transport
                    .write_frame_for(&Response::new(req.id, ManagerResponse::List(list)))
                    .await
                    .unwrap();
            });

            let dest: Destination = "ssh://user@host".parse().unwrap();
            let result = find_existing_connection_id(&mut client, &dest).await;
            assert_eq!(result, None);
        }

        #[tokio::test]
        async fn scheme_matching_is_case_insensitive() {
            let (mut client, mut transport) = setup();

            tokio::spawn(async move {
                let req: Request<ManagerRequest> =
                    transport.read_frame_as().await.unwrap().unwrap();
                let mut list = ConnectionList::new();
                list.insert(42, "SSH://user@host".parse::<Destination>().unwrap());
                transport
                    .write_frame_for(&Response::new(req.id, ManagerResponse::List(list)))
                    .await
                    .unwrap();
            });

            let dest: Destination = "ssh://user@host".parse().unwrap();
            let result = find_existing_connection_id(&mut client, &dest).await;
            assert_eq!(result, Some(42));
        }

        #[tokio::test]
        async fn returns_none_on_list_error() {
            let (mut client, mut transport) = setup();

            tokio::spawn(async move {
                let req: Request<ManagerRequest> =
                    transport.read_frame_as().await.unwrap().unwrap();
                transport
                    .write_frame_for(&Response::new(
                        req.id,
                        ManagerResponse::Error {
                            description: "connection failed".into(),
                        },
                    ))
                    .await
                    .unwrap();
            });

            let dest: Destination = "ssh://user@host".parse().unwrap();
            let result = find_existing_connection_id(&mut client, &dest).await;
            assert_eq!(result, None);
        }
    }
}
