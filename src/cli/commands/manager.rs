use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;

use anyhow::Context;
use distant_core::net::manager::{Config as NetManagerConfig, ConnectHandler, LaunchHandler};
use log::*;
use once_cell::sync::Lazy;
use service_manager::{
    ServiceInstallCtx, ServiceLabel, ServiceLevel, ServiceManager, ServiceStartCtx, ServiceStopCtx,
    ServiceUninstallCtx,
};

use crate::cli::common::{connect_to_manager, Ui};
use crate::cli::Manager;
use crate::options::{Format, ManagerServiceSubcommand, ManagerSubcommand};
#[cfg(unix)]
use crate::CliError;
use crate::CliResult;

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
    }
}
