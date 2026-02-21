use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::Context;
use dialoguer::console::Term;
use dialoguer::theme::ColorfulTheme;
use dialoguer::Select;
use distant_core::net::common::{ConnectionId, Destination};
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

use crate::cli::common::{connect_to_manager, MsgReceiver, MsgSender, Ui};
use crate::cli::{Cache, Manager};
use crate::options::{Format, ManagerServiceSubcommand, ManagerSubcommand};
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
    let ui = Ui::new();

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
        ManagerSubcommand::Service(ManagerServiceSubcommand::Install {
            kind,
            user,
            args: extra_args,
        }) => {
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

            for arg in extra_args {
                args.push(arg.into());
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
            #[cfg_attr(not(unix), allow(unused_variables))]
            access,
            daemon: _daemon,
            network,
            user,
        } => {
            #[cfg(unix)]
            let access = access.unwrap_or_default();

            info!(
                "Starting manager (network = {})",
                if let Some(pipe) = network.windows_pipe.as_ref().filter(|_| cfg!(windows)) {
                    format!("custom:windows:{}", pipe)
                } else if let Some(socket) = network.unix_socket.as_ref().filter(|_| cfg!(unix)) {
                    format!("custom:unix:{:?}", socket)
                } else if user {
                    "user".to_string()
                } else {
                    "global".to_string()
                }
            );
            let manager = Manager {
                #[cfg(unix)]
                access,
                config: NetManagerConfig {
                    user,
                    launch_handlers: {
                        let mut handlers: HashMap<String, Box<dyn LaunchHandler>> = HashMap::new();
                        handlers.insert(
                            "manager".to_string(),
                            Box::new(handlers::ManagerLaunchHandler::new()),
                        );

                        handlers.insert("ssh".to_string(), Box::new(handlers::SshLaunchHandler));

                        handlers
                    },
                    connect_handlers: {
                        let mut handlers: HashMap<String, Box<dyn ConnectHandler>> = HashMap::new();

                        handlers.insert(
                            "distant".to_string(),
                            Box::new(handlers::DistantConnectHandler),
                        );

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
            manager.await.context("Failed to wait on manager")?;
            info!("Manager is shutting down");

            Ok(())
        }
        ManagerSubcommand::Version { format, network } => {
            debug!("Connecting to manager");
            let mut client = connect_to_manager(format, network, &ui).await?;

            debug!("Getting version");
            let version = client.version().await.context("Failed to get version")?;
            debug!("Got version: {version}");

            match format {
                Format::Json => {
                    println!(
                        "{}",
                        serde_json::to_string(&serde_json::json!({ "version": version }))
                            .context("Failed to format version as json")?
                    );
                }
                Format::Shell => {
                    println!("{version}");
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
            let mut client = connect_to_manager(format, network, &ui).await?;

            let id = match id {
                Some(id) => id,
                None => {
                    match prompt_for_connection(&mut client, format, "Select connection to inspect")
                        .await?
                    {
                        Some(id) => id,
                        None => return Ok(()),
                    }
                }
            };

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
            let mut client = connect_to_manager(format, network, &ui).await?;

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
            cache,
            format,
            id,
            network,
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
                                     distant manager list"
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

/// Format a connection list entry for display in selection prompts.
fn format_connection(id: ConnectionId, dest: &Destination) -> String {
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
}

/// Prompt the user to select a connection from the manager's active list.
/// Returns the selected connection ID, or `None` if the user cancels (Escape).
/// Errors if no connections exist or in non-interactive/JSON mode with multiple.
async fn prompt_for_connection(
    client: &mut ManagerClient,
    format: Format,
    prompt_text: &str,
) -> anyhow::Result<Option<ConnectionId>> {
    let list = client
        .list()
        .await
        .context("Failed to get list of connections")?;

    if list.is_empty() {
        anyhow::bail!(
            "No active connections.\n\n\
             Connect to a remote host first:\n  \
             distant connect ssh://user@host\n  \
             distant ssh user@host"
        );
    }

    if list.len() == 1 {
        return Ok(Some(*list.keys().next().unwrap()));
    }

    // Multiple connections — need interactive selection
    let items: Vec<String> = list
        .iter()
        .map(|(id, dest)| format_connection(*id, dest))
        .collect();

    match format {
        Format::Shell => {
            if !Term::stderr().is_term() {
                anyhow::bail!(
                    "Multiple active connections. Specify the connection ID:\n  \
                     distant manager list    (see available connections)"
                );
            }
            let selection = Select::with_theme(&ColorfulTheme::default())
                .with_prompt(prompt_text)
                .items(&items)
                .default(0)
                .interact_on_opt(&Term::stderr())
                .context("Failed to render prompt")?;
            match selection {
                Some(index) => Ok(Some(*list.keys().nth(index).unwrap())),
                None => Ok(None),
            }
        }
        Format::Json => {
            anyhow::bail!("Connection ID is required in JSON mode");
        }
    }
}
