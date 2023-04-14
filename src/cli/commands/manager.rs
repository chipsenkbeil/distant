use crate::options::{ManagerServiceSubcommand, ManagerSubcommand};
use crate::{
    cli::{Cache, Client, Manager},
    CliResult,
};
use anyhow::Context;
use distant_core::net::common::ConnectionId;
use distant_core::net::manager::{Config as NetManagerConfig, ConnectHandler, LaunchHandler};
use log::*;
use once_cell::sync::Lazy;
use service_manager::{
    ServiceInstallCtx, ServiceLabel, ServiceLevel, ServiceManager, ServiceStartCtx, ServiceStopCtx,
    ServiceUninstallCtx,
};
use std::{collections::HashMap, ffi::OsString, path::PathBuf};
use tabled::{Table, Tabled};

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
    use crate::CliError;
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
            network,
            user,
            ..
        } => {
            let access = access.unwrap_or_default();

            info!(
                "Starting manager (network = {})",
                if (cfg!(windows) && network.windows_pipe.is_some())
                    || (cfg!(unix) && network.unix_socket.is_some())
                {
                    "custom"
                } else if user {
                    "user"
                } else {
                    "global"
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
        ManagerSubcommand::Capabilities { network } => {
            debug!("Getting list of capabilities");
            let caps = Client::new(network)
                .using_prompt_auth_handler()
                .connect()
                .await
                .context("Failed to connect to manager")?
                .capabilities()
                .await
                .context("Failed to get list of capabilities")?;
            debug!("Got capabilities: {caps:?}");

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

            Ok(())
        }
        ManagerSubcommand::Info { network, id } => {
            debug!("Getting info about connection {}", id);
            let info = Client::new(network)
                .using_prompt_auth_handler()
                .connect()
                .await
                .context("Failed to connect to manager")?
                .info(id)
                .await
                .context("Failed to get info about connection")?;
            debug!("Got info: {info:?}");

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

            Ok(())
        }
        ManagerSubcommand::List { network, cache } => {
            debug!("Getting list of connections");
            let list = Client::new(network)
                .using_prompt_auth_handler()
                .connect()
                .await
                .context("Failed to connect to manager")?
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

            Ok(())
        }
        ManagerSubcommand::Kill { network, id } => {
            debug!("Killing connection {}", id);
            Client::new(network)
                .using_prompt_auth_handler()
                .connect()
                .await
                .context("Failed to connect to manager")?
                .kill(id)
                .await
                .with_context(|| format!("Failed to kill connection to server {id}"))?;
            debug!("Connection killed");
            Ok(())
        }
    }
}
