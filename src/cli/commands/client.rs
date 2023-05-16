use std::io;
use std::io::Write;
use std::path::Path;
use std::time::Duration;

use anyhow::Context;
use distant_core::data::{ChangeKindSet, FileType, SearchQuery, SystemInfo};
use distant_core::net::common::{ConnectionId, Host, Map, Request, Response};
use distant_core::net::manager::ManagerClient;
use distant_core::{
    DistantChannel, DistantChannelExt, DistantMsg, DistantRequestData, DistantResponseData,
    RemoteCommand, Searcher, Watcher,
};
use log::*;
use serde_json::json;
use tabled::object::Rows;
use tabled::style::Style;
use tabled::{Alignment, Disable, Modify, Table, Tabled};
use tokio::sync::mpsc;

use crate::cli::common::{
    Cache, Client, JsonAuthHandler, MsgReceiver, MsgSender, PromptAuthHandler,
};
use crate::constants::MAX_PIPE_CHUNK_SIZE;
use crate::options::{ClientFileSystemSubcommand, ClientSubcommand, Format, NetworkSettings};
use crate::{CliError, CliResult};

mod lsp;
mod shell;

use lsp::Lsp;
use shell::Shell;

use super::common::{Formatter, RemoteProcessLink};

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
        ClientSubcommand::Capabilities {
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

            debug!("Retrieving capabilities");
            let capabilities = channel
                .into_client()
                .into_channel()
                .capabilities()
                .await
                .with_context(|| {
                    format!("Failed to retrieve capabilities using connection {connection_id}")
                })?;

            match format {
                Format::Shell => {
                    #[derive(Tabled)]
                    struct EntryRow {
                        kind: String,
                        description: String,
                    }

                    let table = Table::new(capabilities.into_sorted_vec().into_iter().map(|cap| {
                        EntryRow {
                            kind: cap.kind,
                            description: cap.description,
                        }
                    }))
                    .with(Style::ascii())
                    .with(Modify::new(Rows::new(..)).with(Alignment::left()))
                    .to_string();

                    println!("{table}");
                }
                Format::Json => println!("{}", serde_json::to_string(&capabilities).unwrap()),
            }
        }
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
            let mut client = Client::new(network)
                .using_json_auth_handler()
                .connect()
                .await
                .context("Failed to connect to manager")?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            let timeout = match timeout {
                Some(timeout) if timeout >= f32::EPSILON => Some(timeout),
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
                let mut rx =
                    MsgReceiver::from_stdin().into_rx::<Request<DistantMsg<DistantRequestData>>>();
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
                            .try_read_frame_as::<Response<DistantMsg<DistantResponseData>>>()
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
            let mut client = Client::new(network)
                .using_prompt_auth_handler()
                .connect()
                .await
                .context("Failed to connect to manager")?;

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
                .spawn(cmd, environment, current_dir, MAX_PIPE_CHUNK_SIZE)
                .await?;
        }
        ClientSubcommand::Spawn {
            cache,
            connection,
            cmd,
            current_dir,
            environment,
            lsp,
            pty,
            network,
        } => {
            debug!("Connecting to manager");
            let mut client = Client::new(network)
                .using_prompt_auth_handler()
                .connect()
                .await
                .context("Failed to connect to manager")?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            debug!("Opening channel to connection {}", connection_id);
            let channel = client
                .open_raw_channel(connection_id)
                .await
                .with_context(|| format!("Failed to open channel to connection {connection_id}"))?;

            // Convert cmd into string
            let cmd = cmd.join(" ");

            if lsp {
                debug!(
                    "Spawning LSP server (pty = {}, cwd = {:?}): {}",
                    pty, current_dir, cmd
                );
                Lsp::new(channel.into_client().into_channel())
                    .spawn(cmd, current_dir, pty, MAX_PIPE_CHUNK_SIZE)
                    .await?;
            } else if pty {
                debug!(
                    "Spawning pty process (environment = {:?}, cwd = {:?}): {}",
                    environment, current_dir, cmd
                );
                Shell::new(channel.into_client().into_channel())
                    .spawn(cmd, environment, current_dir, MAX_PIPE_CHUNK_SIZE)
                    .await?;
            } else {
                debug!(
                    "Spawning regular process (environment = {:?}, cwd = {:?}): {}",
                    environment, current_dir, cmd
                );
                let mut proc = RemoteCommand::new()
                    .environment(environment)
                    .current_dir(current_dir)
                    .pty(None)
                    .spawn(channel.into_client().into_channel(), &cmd)
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
            let mut client = Client::new(network)
                .using_prompt_auth_handler()
                .connect()
                .await
                .context("Failed to connect to manager")?;

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
        ClientSubcommand::FileSystem(ClientFileSystemSubcommand::Copy {
            cache,
            connection,
            network,
            src,
            dst,
        }) => {
            debug!("Connecting to manager");
            let mut client = Client::new(network)
                .using_prompt_auth_handler()
                .connect()
                .await
                .context("Failed to connect to manager")?;

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
            let mut client = Client::new(network)
                .using_prompt_auth_handler()
                .connect()
                .await
                .context("Failed to connect to manager")?;

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
            let mut client = Client::new(network)
                .using_prompt_auth_handler()
                .connect()
                .await
                .context("Failed to connect to manager")?;

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
            let mut client = Client::new(network)
                .using_prompt_auth_handler()
                .connect()
                .await
                .context("Failed to connect to manager")?;

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
            let mut client = Client::new(network)
                .using_prompt_auth_handler()
                .connect()
                .await
                .context("Failed to connect to manager")?;

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
                .send(DistantMsg::Batch(vec![
                    DistantRequestData::FileRead {
                        path: path.to_path_buf(),
                    },
                    DistantRequestData::DirRead {
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
                    DistantResponseData::DirEntries { entries, .. } => {
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
                    DistantResponseData::Blob { data } => {
                        let mut out = std::io::stdout();
                        out.write_all(&data)
                            .context("Failed to write file contents to stdout")?;
                        out.flush().context("Failed to flush stdout")?;
                        return Ok(());
                    }
                    DistantResponseData::Error(x) => errors.push(x),
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
            let mut client = Client::new(network)
                .using_prompt_auth_handler()
                .connect()
                .await
                .context("Failed to connect to manager")?;

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
            let mut client = Client::new(network)
                .using_prompt_auth_handler()
                .connect()
                .await
                .context("Failed to connect to manager")?;

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
            let mut client = Client::new(network)
                .using_prompt_auth_handler()
                .connect()
                .await
                .context("Failed to connect to manager")?;

            let mut cache = read_cache(&cache).await;
            let connection_id =
                use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

            debug!("Opening channel to connection {}", connection_id);
            let channel = client
                .open_raw_channel(connection_id)
                .await
                .with_context(|| format!("Failed to open channel to connection {connection_id}"))?;

            let mut formatter = Formatter::shell();
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
            while let Some(m) = searcher.next().await {
                // TODO: Provide a cleaner way to print just a match
                let res = Response::new(
                    "".to_string(),
                    DistantMsg::Single(DistantResponseData::SearchResults {
                        id: 0,
                        matches: vec![m],
                    }),
                );

                formatter.print(res).context("Failed to print match")?;
            }
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
            let mut client = Client::new(network)
                .using_prompt_auth_handler()
                .connect()
                .await
                .context("Failed to connect to manager")?;

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
            let mut formatter = Formatter::shell();
            while let Some(change) = watcher.next().await {
                // TODO: Provide a cleaner way to print just a change
                let res = Response::new(
                    "".to_string(),
                    DistantMsg::Single(DistantResponseData::Changed(change)),
                );

                formatter.print(res).context("Failed to print change")?;
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
            let mut client = Client::new(network)
                .using_prompt_auth_handler()
                .connect()
                .await
                .context("Failed to connect to manager")?;

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
                anyhow::bail!("There are no connections being managed! You need to start one!");
            } else if list.len() > 1 {
                trace!("Cached connection id is invalid and there are multiple connections");
                anyhow::bail!(
                    "There are multiple connections being managed! You need to pick one!"
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

async fn connect_to_manager(
    format: Format,
    network: NetworkSettings,
) -> anyhow::Result<ManagerClient> {
    Ok(match format {
        Format::Shell => Client::new(network)
            .using_prompt_auth_handler()
            .connect()
            .await
            .context("Failed to connect to manager")?,
        Format::Json => Client::new(network)
            .using_json_auth_handler()
            .connect()
            .await
            .context("Failed to connect to manager")?,
    })
}
