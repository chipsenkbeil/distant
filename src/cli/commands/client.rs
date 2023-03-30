use crate::{
    cli::{
        client::{JsonAuthHandler, MsgReceiver, MsgSender, PromptAuthHandler},
        Cache, Client,
    },
    config::{
        ClientActionConfig, ClientConfig, ClientConnectConfig, ClientLaunchConfig,
        ClientReplConfig, NetworkConfig,
    },
    paths::user::CACHE_FILE_PATH_STR,
    CliError, CliResult,
};
use anyhow::Context;
use clap::{Subcommand, ValueHint};
use dialoguer::{console::Term, theme::ColorfulTheme, Select};
use distant_core::{
    data::{ChangeKindSet, Environment},
    net::common::{ConnectionId, Destination, Host, Map, Request, Response},
    net::manager::ManagerClient,
    DistantMsg, DistantRequestData, DistantResponseData, RemoteCommand, Searcher, Watcher,
};
use log::*;
use serde_json::{json, Value};
use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::sync::mpsc;

mod buf;
mod format;
mod link;
mod lsp;
mod shell;
mod stdin;

pub use format::Format;
use format::Formatter;
use link::RemoteProcessLink;
use lsp::Lsp;
use shell::Shell;

const SLEEP_DURATION: Duration = Duration::from_millis(1);

#[derive(Debug, Subcommand)]
pub enum ClientSubcommand {
    /// Performs some action on a remote machine
    Action {
        /// Location to store cached data
        #[clap(
            long,
            value_hint = ValueHint::FilePath,
            value_parser,
            default_value = CACHE_FILE_PATH_STR.as_str()
        )]
        cache: PathBuf,

        #[clap(flatten)]
        config: ClientActionConfig,

        /// Specify a connection being managed
        #[clap(long)]
        connection: Option<ConnectionId>,

        #[clap(flatten)]
        network: NetworkConfig,

        #[clap(subcommand)]
        request: DistantRequestData,
    },

    /// Requests that active manager connects to the server at the specified destination
    Connect {
        /// Location to store cached data
        #[clap(
            long,
            value_hint = ValueHint::FilePath,
            value_parser,
            default_value = CACHE_FILE_PATH_STR.as_str()
        )]
        cache: PathBuf,

        #[clap(flatten)]
        config: ClientConnectConfig,

        #[clap(flatten)]
        network: NetworkConfig,

        #[clap(short, long, default_value_t, value_enum)]
        format: Format,

        destination: Box<Destination>,
    },

    /// Launches the server-portion of the binary on a remote machine
    Launch {
        /// Location to store cached data
        #[clap(
            long,
            value_hint = ValueHint::FilePath,
            value_parser,
            default_value = CACHE_FILE_PATH_STR.as_str()
        )]
        cache: PathBuf,

        #[clap(flatten)]
        config: ClientLaunchConfig,

        #[clap(flatten)]
        network: NetworkConfig,

        #[clap(short, long, default_value_t, value_enum)]
        format: Format,

        destination: Box<Destination>,
    },

    /// Specialized treatment of running a remote LSP process
    Lsp {
        /// Location to store cached data
        #[clap(
            long,
            value_hint = ValueHint::FilePath,
            value_parser,
            default_value = CACHE_FILE_PATH_STR.as_str()
        )]
        cache: PathBuf,

        /// Specify a connection being managed
        #[clap(long)]
        connection: Option<ConnectionId>,

        #[clap(flatten)]
        network: NetworkConfig,

        /// Alternative current directory for the remote process
        #[clap(long)]
        current_dir: Option<PathBuf>,

        /// If provided, will run LSP in a pty
        #[clap(long)]
        pty: bool,

        cmd: String,
    },

    /// Runs actions in a read-eval-print loop
    Repl {
        /// Location to store cached data
        #[clap(
            long,
            value_hint = ValueHint::FilePath,
            value_parser,
            default_value = CACHE_FILE_PATH_STR.as_str()
        )]
        cache: PathBuf,

        #[clap(flatten)]
        config: ClientReplConfig,

        /// Specify a connection being managed
        #[clap(long)]
        connection: Option<ConnectionId>,

        #[clap(flatten)]
        network: NetworkConfig,

        /// Format used for input into and output from the repl
        #[clap(short, long, default_value_t, value_enum)]
        format: Format,
    },

    /// Select the active connection
    Select {
        /// Location to store cached data
        #[clap(
            long,
            value_hint = ValueHint::FilePath,
            value_parser,
            default_value = CACHE_FILE_PATH_STR.as_str()
        )]
        cache: PathBuf,

        /// Connection to use, otherwise will prompt to select
        connection: Option<ConnectionId>,

        #[clap(short, long, default_value_t, value_enum)]
        format: Format,

        #[clap(flatten)]
        network: NetworkConfig,
    },

    /// Specialized treatment of running a remote shell process
    Shell {
        /// Location to store cached data
        #[clap(
            long,
            value_hint = ValueHint::FilePath,
            value_parser,
            default_value = CACHE_FILE_PATH_STR.as_str()
        )]
        cache: PathBuf,

        /// Specify a connection being managed
        #[clap(long)]
        connection: Option<ConnectionId>,

        #[clap(flatten)]
        network: NetworkConfig,

        /// Alternative current directory for the remote process
        #[clap(long)]
        current_dir: Option<PathBuf>,

        /// Environment variables to provide to the shell
        #[clap(long, default_value_t)]
        environment: Environment,

        /// Optional command to run instead of $SHELL
        cmd: Option<String>,
    },
}

impl ClientSubcommand {
    pub fn run(self, config: ClientConfig) -> CliResult {
        let rt = tokio::runtime::Runtime::new().context("Failed to start up runtime")?;
        rt.block_on(Self::async_run(self, config))
    }

    fn cache_path(&self) -> &Path {
        match self {
            Self::Action { cache, .. } => cache.as_path(),
            Self::Connect { cache, .. } => cache.as_path(),
            Self::Launch { cache, .. } => cache.as_path(),
            Self::Lsp { cache, .. } => cache.as_path(),
            Self::Repl { cache, .. } => cache.as_path(),
            Self::Select { cache, .. } => cache.as_path(),
            Self::Shell { cache, .. } => cache.as_path(),
        }
    }

    async fn async_run(self, config: ClientConfig) -> CliResult {
        // If we get an error, just default anyway
        let mut cache = Cache::read_from_disk_or_default(self.cache_path().to_path_buf())
            .await
            .unwrap_or_else(|_| Cache::new(self.cache_path().to_path_buf()));

        match self {
            Self::Action {
                config: action_config,
                connection,
                network,
                request,
                ..
            } => {
                let network = network.merge(config.network);
                debug!("Connecting to manager");
                let mut client = Client::new(network)
                    .using_prompt_auth_handler()
                    .connect()
                    .await
                    .context("Failed to connect to manager")?;

                let connection_id =
                    use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

                debug!("Opening channel to connection {}", connection_id);
                let channel = client
                    .open_raw_channel(connection_id)
                    .await
                    .with_context(|| {
                        format!("Failed to open channel to connection {connection_id}")
                    })?;

                let timeout = action_config.timeout.or(config.action.timeout);

                debug!(
                    "Timeout configured to be {}",
                    match timeout {
                        Some(secs) => format!("{}s", secs),
                        None => "none".to_string(),
                    }
                );

                let mut formatter = Formatter::shell();

                debug!("Sending request {:?}", request);
                match request {
                    DistantRequestData::ProcSpawn {
                        cmd,
                        environment,
                        current_dir,
                        pty,
                    } => {
                        debug!("Special request spawning {:?}", cmd);
                        let mut proc = RemoteCommand::new()
                            .environment(environment)
                            .current_dir(current_dir)
                            .pty(pty)
                            .spawn(channel.into_client().into_channel(), cmd.as_str())
                            .await
                            .with_context(|| format!("Failed to spawn {cmd}"))?;

                        // Now, map the remote process' stdin/stdout/stderr to our own process
                        let link = RemoteProcessLink::from_remote_pipes(
                            proc.stdin.take(),
                            proc.stdout.take().unwrap(),
                            proc.stderr.take().unwrap(),
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
                    DistantRequestData::Search { query } => {
                        debug!("Special request creating searcher for {:?}", query);
                        let mut searcher =
                            Searcher::search(channel.into_client().into_channel(), query)
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
                    DistantRequestData::Watch {
                        path,
                        recursive,
                        only,
                        except,
                    } => {
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
                            // TODO: Provide a cleaner way to print just a change
                            let res = Response::new(
                                "".to_string(),
                                DistantMsg::Single(DistantResponseData::Changed(change)),
                            );

                            formatter.print(res).context("Failed to print change")?;
                        }
                    }
                    request => {
                        let response = channel
                            .into_client()
                            .into_channel()
                            .send_timeout(
                                DistantMsg::Single(request),
                                timeout
                                    .or(config.action.timeout)
                                    .map(Duration::from_secs_f32),
                            )
                            .await
                            .context("Failed to send request")?;

                        debug!("Got response {:?}", response);

                        // NOTE: We expect a single response, and if that is an error then
                        //       we want to pass that error up the stack
                        let id = response.id;
                        let origin_id = response.origin_id;
                        match response.payload {
                            DistantMsg::Single(DistantResponseData::Error(x)) => {
                                return Err(CliError::Error(anyhow::anyhow!(x)));
                            }
                            payload => formatter
                                .print(Response {
                                    id,
                                    origin_id,
                                    payload,
                                })
                                .context("Failed to print response")?,
                        }
                    }
                }
            }
            Self::Connect {
                config: connect_config,
                network,
                format,
                destination,
                ..
            } => {
                let network = network.merge(config.network);
                debug!("Connecting to manager");
                let mut client = match format {
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
                };

                // Merge our connect configs, overwriting anything in the config file with our cli
                // arguments
                let mut options = Map::from(config.connect);
                options.extend(Map::from(connect_config).into_map());

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
                *cache.data.selected = id;
                cache.write_to_disk().await?;

                match format {
                    Format::Shell => println!("{}", id),
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
            Self::Launch {
                config: launch_config,
                network,
                format,
                mut destination,
                ..
            } => {
                let network = network.merge(config.network);
                debug!("Connecting to manager");
                let mut client = match format {
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
                };

                // Merge our launch configs, overwriting anything in the config file
                // with our cli arguments
                let mut options = Map::from(config.launch);
                options.extend(Map::from(launch_config).into_map());

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
                *cache.data.selected = id;
                cache.write_to_disk().await?;

                match format {
                    Format::Shell => println!("{}", id),
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
            Self::Lsp {
                connection,
                network,
                current_dir,
                pty,
                cmd,
                ..
            } => {
                let network = network.merge(config.network);
                debug!("Connecting to manager");
                let mut client = Client::new(network)
                    .using_prompt_auth_handler()
                    .connect()
                    .await
                    .context("Failed to connect to manager")?;

                let connection_id =
                    use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

                debug!("Opening channel to connection {}", connection_id);
                let channel = client
                    .open_raw_channel(connection_id)
                    .await
                    .with_context(|| {
                        format!("Failed to open channel to connection {connection_id}")
                    })?;

                debug!("Spawning LSP server (pty = {}): {}", pty, cmd);
                Lsp::new(channel.into_client().into_channel())
                    .spawn(cmd, current_dir, pty)
                    .await?;
            }
            Self::Repl {
                config: repl_config,
                connection,
                network,
                format,
                ..
            } => {
                // TODO: Support shell format?
                if !format.is_json() {
                    return Err(CliError::Error(anyhow::anyhow!(
                        "Only JSON format is supported"
                    )));
                }

                let network = network.merge(config.network);
                debug!("Connecting to manager");
                let mut client = Client::new(network)
                    .using_json_auth_handler()
                    .connect()
                    .await
                    .context("Failed to connect to manager")?;

                let connection_id =
                    use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

                let timeout = repl_config.timeout.or(config.repl.timeout);

                debug!("Opening raw channel to connection {}", connection_id);
                let mut channel =
                    client
                        .open_raw_channel(connection_id)
                        .await
                        .with_context(|| {
                            format!("Failed to open raw channel to connection {connection_id}")
                        })?;

                debug!(
                    "Timeout configured to be {}",
                    match timeout {
                        Some(secs) => format!("{}s", secs),
                        None => "none".to_string(),
                    }
                );

                debug!("Starting repl using format {:?}", format);
                let (msg_tx, mut msg_rx) = mpsc::channel(1);
                let request_task = tokio::spawn(async move {
                    let mut rx = MsgReceiver::from_stdin()
                        .into_rx::<Request<DistantMsg<DistantRequestData>>>();
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
            Self::Select {
                connection,
                format,
                network,
                ..
            } => match connection {
                Some(id) => {
                    *cache.data.selected = id;
                    cache.write_to_disk().await?;
                }
                None => {
                    let network = network.merge(config.network);
                    debug!("Connecting to manager");
                    let mut client = match format {
                        Format::Json => Client::new(network)
                            .using_json_auth_handler()
                            .connect()
                            .await
                            .context("Failed to connect to manager")?,
                        Format::Shell => Client::new(network)
                            .using_prompt_auth_handler()
                            .connect()
                            .await
                            .context("Failed to connect to manager")?,
                    };
                    let list = client
                        .list()
                        .await
                        .context("Failed to get a list of managed connections")?;

                    if list.is_empty() {
                        return Err(CliError::Error(anyhow::anyhow!(
                            "No connection available in manager"
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
                        .map(|(_, destination)| {
                            format!(
                                "{}{}{}",
                                destination
                                    .scheme
                                    .as_ref()
                                    .map(|scheme| format!(r"{scheme}://"))
                                    .unwrap_or_default(),
                                destination.host,
                                destination
                                    .port
                                    .map(|port| format!(":{port}"))
                                    .unwrap_or_default()
                            )
                        })
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
                                .recv_blocking::<Value>()
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
                                    )))
                                }
                                None => {
                                    return Err(CliError::Error(anyhow::anyhow!(
                                        "Missing 'type' field"
                                    )))
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
            },
            Self::Shell {
                connection,
                network,
                current_dir,
                environment,
                cmd,
                ..
            } => {
                let network = network.merge(config.network);
                debug!("Connecting to manager");
                let mut client = Client::new(network)
                    .using_prompt_auth_handler()
                    .connect()
                    .await
                    .context("Failed to connect to manager")?;

                let connection_id =
                    use_or_lookup_connection_id(&mut cache, connection, &mut client).await?;

                debug!("Opening channel to connection {}", connection_id);
                let channel = client
                    .open_raw_channel(connection_id)
                    .await
                    .with_context(|| {
                        format!("Failed to open channel to connection {connection_id}")
                    })?;

                debug!(
                    "Spawning shell (environment = {:?}): {}",
                    environment,
                    cmd.as_deref().unwrap_or(r"$SHELL")
                );
                Shell::new(channel.into_client().into_channel())
                    .spawn(cmd, environment, current_dir)
                    .await?;
            }
        }

        Ok(())
    }
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
