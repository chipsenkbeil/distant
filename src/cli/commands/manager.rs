use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::Context;
use dialoguer::console::Term;
use dialoguer::theme::ColorfulTheme;
use dialoguer::Select;
use distant_core::net::common::ConnectionId;
use distant_core::net::manager::{
    Config as NetManagerConfig, ConnectHandler, LaunchHandler, ManagerClient,
};
use log::*;
use once_cell::sync::Lazy;
use serde_json::{json, Value};
use service_manager::{
    ServiceInstallCtx, ServiceLabel, ServiceLevel, ServiceManager, ServiceStartCtx, ServiceStopCtx,
    ServiceUninstallCtx,
};
use tabled::{Table, Tabled};

use crate::cli::common::{MsgReceiver, MsgSender};
use crate::cli::{Cache, Client, Manager};
use crate::options::{Format, ManagerServiceSubcommand, ManagerSubcommand, NetworkSettings};
use crate::{CliError, CliResult};

/// [`ServiceLabel`] for our manager in the form `rocks.distant.manager`
static SERVICE_LABEL: Lazy<ServiceLabel> = Lazy::new(|| ServiceLabel {
    qualifier: String::from("rocks"),
    organization: String::from("distant"),
    application: String::from("manager"),
});

mod handlers;

pub fn run(cmd: ManagerSubcommand) -> CliResult {
    match &cmd {
        ManagerSubcommand::Listen { daemon, .. } if *daemon => run_daemon(cmd),
        _ => {
            let rt = tokio::runtime::Runtime::new().context("Failed to start up runtime")?;
            rt.block_on(async_run(cmd))
        }
    }
}

#[cfg(windows)]
fn run_daemon(_cmd: ManagerSubcommand) -> CliResult {
    use crate::cli::Spawner;
    let pid = Spawner::spawn_running_background(Vec::new())
        .context("Failed to spawn background process")?;
    println!("[distant manager detached, pid = {}]", pid);
    Ok(())
}

#[cfg(unix)]
fn run_daemon(cmd: ManagerSubcommand) -> CliResult {
    use fork::{daemon, Fork};

    debug!("Forking process");
    match daemon(true, true) {
        Ok(Fork::Child) => {
            let rt = tokio::runtime::Runtime::new().context("Failed to start up runtime")?;
            rt.block_on(async { async_run(cmd).await })?;
            Ok(())
        }
        Ok(Fork::Parent(pid)) => {
            println!("[distant manager detached, pid = {pid}]");
            if fork::close_fd().is_err() {
                Err(CliError::Error(anyhow::anyhow!("Fork failed to close fd")))
            } else {
                Ok(())
            }
        }
        Err(_) => Err(CliError::Error(anyhow::anyhow!("Fork failed"))),
    }
}

async fn async_run(cmd: ManagerSubcommand) -> CliResult {
    match cmd {
        ManagerSubcommand::Service(ManagerServiceSubcommand::Start { kind, user }) => {
            debug!("Starting manager service via {:?}", kind);
            let mut manager = <dyn ServiceManager>::target_or_native(kind)
                .context("Failed to detect native service manager")?;

            if user {
                manager
                    .set_level(ServiceLevel::User)
                    .context("Failed to set service manager to user level")?;
            }

            manager
                .start(ServiceStartCtx {
                    label: SERVICE_LABEL.clone(),
                })
                .context("Failed to start service")?;
            Ok(())
        }
        ManagerSubcommand::Service(ManagerServiceSubcommand::Stop { kind, user }) => {
            debug!("Stopping manager service via {:?}", kind);
            let mut manager = <dyn ServiceManager>::target_or_native(kind)
                .context("Failed to detect native service manager")?;

            if user {
                manager
                    .set_level(ServiceLevel::User)
                    .context("Failed to set service manager to user level")?;
            }

            manager
                .stop(ServiceStopCtx {
                    label: SERVICE_LABEL.clone(),
                })
                .context("Failed to stop service")?;
            Ok(())
        }
        ManagerSubcommand::Service(ManagerServiceSubcommand::Install { kind, user }) => {
            debug!("Installing manager service via {:?}", kind);
            let mut manager = <dyn ServiceManager>::target_or_native(kind)
                .context("Failed to detect native service manager")?;
            let mut args = vec![OsString::from("manager"), OsString::from("listen")];

            if user {
                args.push(OsString::from("--user"));
                manager
                    .set_level(ServiceLevel::User)
                    .context("Failed to set service manager to user level")?;
            }

            manager
                .install(ServiceInstallCtx {
                    label: SERVICE_LABEL.clone(),

                    // distant manager listen
                    program: std::env::current_exe()
                        .ok()
                        .unwrap_or_else(|| PathBuf::from("distant")),
                    args,
                })
                .context("Failed to install service")?;

            Ok(())
        }
        ManagerSubcommand::Service(ManagerServiceSubcommand::Uninstall { kind, user }) => {
            debug!("Uninstalling manager service via {:?}", kind);
            let mut manager = <dyn ServiceManager>::target_or_native(kind)
                .context("Failed to detect native service manager")?;
            if user {
                manager
                    .set_level(ServiceLevel::User)
                    .context("Failed to set service manager to user level")?;
            }
            manager
                .uninstall(ServiceUninstallCtx {
                    label: SERVICE_LABEL.clone(),
                })
                .context("Failed to uninstall service")?;

            Ok(())
        }
        ManagerSubcommand::Listen {
            access,
            daemon: _daemon,
            network,
            user,
        } => {
            let access = access.unwrap_or_default();

            info!(
                "Starting manager (network = {})",
                if cfg!(windows) && network.windows_pipe.is_some() {
                    format!("custom:windows:{}", network.windows_pipe.as_ref().unwrap())
                } else if cfg!(unix) && network.unix_socket.is_some() {
                    format!("custom:unix:{:?}", network.unix_socket.as_ref().unwrap())
                } else if user {
                    "user".to_string()
                } else {
                    "global".to_string()
                }
            );
            let manager_ref = Manager {
                access,
                config: NetManagerConfig {
                    user,
                    launch_handlers: {
                        let mut handlers: HashMap<String, Box<dyn LaunchHandler>> = HashMap::new();
                        handlers.insert(
                            "manager".to_string(),
                            Box::new(handlers::ManagerLaunchHandler::new()),
                        );

                        #[cfg(any(feature = "libssh", feature = "ssh2"))]
                        handlers.insert("ssh".to_string(), Box::new(handlers::SshLaunchHandler));

                        handlers
                    },
                    connect_handlers: {
                        let mut handlers: HashMap<String, Box<dyn ConnectHandler>> = HashMap::new();

                        handlers.insert(
                            "distant".to_string(),
                            Box::new(handlers::DistantConnectHandler),
                        );

                        #[cfg(any(feature = "libssh", feature = "ssh2"))]
                        handlers.insert("ssh".to_string(), Box::new(handlers::SshConnectHandler));

                        handlers
                    },
                    ..Default::default()
                },
                network,
            }
            .listen()
            .await
            .context("Failed to start manager")?;

            // Let our server run to completion
            manager_ref
                .as_ref()
                .polling_wait()
                .await
                .context("Failed to wait on manager")?;
            info!("Manager is shutting down");

            Ok(())
        }
        ManagerSubcommand::Capabilities { format, network } => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(format, network).await?;

            debug!("Getting list of capabilities");
            let caps = client
                .capabilities()
                .await
                .context("Failed to get list of capabilities")?;
            debug!("Got capabilities: {caps:?}");

            match format {
                Format::Json => {
                    println!(
                        "{}",
                        serde_json::to_string(&caps)
                            .context("Failed to format capabilities as json")?
                    );
                }
                Format::Shell => {
                    #[derive(Tabled)]
                    struct CapabilityRow {
                        kind: String,
                        description: String,
                    }

                    println!(
                        "{}",
                        Table::new(caps.into_sorted_vec().into_iter().map(|cap| {
                            CapabilityRow {
                                kind: cap.kind,
                                description: cap.description,
                            }
                        }))
                    );
                }
            }

            Ok(())
        }
        ManagerSubcommand::Info {
            format,
            id,
            network,
        } => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(format, network).await?;

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
                    #[derive(Tabled)]
                    struct InfoRow {
                        id: ConnectionId,
                        scheme: String,
                        host: String,
                        port: String,
                        options: String,
                    }
                    println!(
                        "{}",
                        Table::new(vec![InfoRow {
                            id: info.id,
                            scheme: info.destination.scheme.unwrap_or_default(),
                            host: info.destination.host.to_string(),
                            port: info
                                .destination
                                .port
                                .map(|x| x.to_string())
                                .unwrap_or_default(),
                            options: info.options.to_string()
                        }])
                    );
                }
            }

            Ok(())
        }
        ManagerSubcommand::List {
            cache,
            format,
            network,
        } => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(format, network).await?;

            debug!("Getting list of connections");
            let list = client
                .list()
                .await
                .context("Failed to get list of connections")?;
            debug!("Got list: {list:?}");

            debug!("Looking up selected connection");
            let selected = Cache::read_from_disk_or_default(cache)
                .await
                .context("Failed to look up selected connection")?
                .data
                .selected;
            debug!("Using selected: {selected}");

            match format {
                Format::Json => {
                    println!(
                        "{}",
                        serde_json::to_string(&list)
                            .context("Failed to format connection list as json")?
                    );
                }
                Format::Shell => {
                    #[derive(Tabled)]
                    struct ListRow {
                        selected: bool,
                        id: ConnectionId,
                        scheme: String,
                        host: String,
                        port: String,
                    }

                    println!(
                        "{}",
                        Table::new(list.into_iter().map(|(id, destination)| {
                            ListRow {
                                selected: *selected == id,
                                id,
                                scheme: destination.scheme.unwrap_or_default(),
                                host: destination.host.to_string(),
                                port: destination.port.map(|x| x.to_string()).unwrap_or_default(),
                            }
                        }))
                    );
                }
            }

            Ok(())
        }
        ManagerSubcommand::Kill {
            format,
            id,
            network,
        } => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(format, network).await?;

            debug!("Killing connection {}", id);
            client
                .kill(id)
                .await
                .with_context(|| format!("Failed to kill connection to server {id}"))?;

            debug!("Connection killed");
            match format {
                Format::Json => println!("{}", json!({"type": "ok"})),
                Format::Shell => (),
            }

            Ok(())
        }
        ManagerSubcommand::Select {
            cache,
            connection,
            format,
            network,
        } => {
            let mut cache = Cache::read_from_disk_or_default(cache)
                .await
                .context("Failed to look up cache")?;

            match connection {
                Some(id) => {
                    *cache.data.selected = id;
                    cache.write_to_disk().await?;
                    Ok(())
                }
                None => {
                    debug!("Connecting to manager");
                    let mut client = connect_to_manager(format, network).await?;
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
                            Ok(())
                        }
                        None => {
                            debug!("No change in selection of default connection id");
                            Ok(())
                        }
                    }
                }
            }
        }
    }
}

async fn connect_to_manager(
    format: Format,
    network: NetworkSettings,
) -> anyhow::Result<ManagerClient> {
    debug!("Connecting to manager");
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
