use crate::{
    cli::{
        CliResult, Client, Manager, Service, ServiceInstallCtx, ServiceKind, ServiceLabel,
        ServiceStartCtx, ServiceStopCtx, ServiceUninstallCtx,
    },
    config::ManagerConfig,
};
use clap::Subcommand;
use distant_core::{net::ServerRef, ConnectionId, DistantManagerConfig};
use log::*;
use once_cell::sync::Lazy;
use std::io;
use tabled::{Table, Tabled};

/// [`ServiceLabel`] for our manager in the form `rocks.distant.manager`
static SERVICE_LABEL: Lazy<ServiceLabel> = Lazy::new(|| ServiceLabel {
    qualifier: String::from("rocks"),
    organization: String::from("distant"),
    application: String::from("manager"),
});

mod handlers;

#[derive(Debug, Subcommand)]
pub enum ManagerSubcommand {
    #[clap(subcommand)]
    Service(ManagerServiceSubcommand),

    /// Listen for incoming requests as a manager
    Listen {
        /// If specified, will fork the process to run as a standalone daemon
        #[clap(long)]
        daemon: bool,
    },

    /// Retrieve information about a specific connection
    Info { id: ConnectionId },

    /// List information about all connections
    List,

    /// Kill a specific connection
    Kill { id: ConnectionId },

    /// Send a shutdown request to the manager
    Shutdown,
}

#[derive(Debug, Subcommand)]
pub enum ManagerServiceSubcommand {
    /// Start the manager as a service
    Start {
        /// Type of service manager used to run this service, defaulting to platform native
        #[clap(long, value_enum)]
        kind: Option<ServiceKind>,
    },

    /// Stop the manager as a service
    Stop {
        #[clap(long, value_enum)]
        kind: Option<ServiceKind>,
    },

    /// Install the manager as a service
    Install {
        #[clap(long, value_enum)]
        kind: Option<ServiceKind>,

        /// If specified, installs as a user-level service
        #[clap(long)]
        user: bool,
    },

    /// Uninstall the manager as a service
    Uninstall {
        #[clap(long, value_enum)]
        kind: Option<ServiceKind>,

        /// If specified, uninstalls a user-level service
        #[clap(long)]
        user: bool,
    },
}

impl ManagerSubcommand {
    pub fn run(self, config: ManagerConfig) -> CliResult<()> {
        match &self {
            Self::Listen { daemon } if *daemon => Self::run_daemon(self, config),
            _ => {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(Self::async_run(self, config))
            }
        }
    }

    #[cfg(windows)]
    fn run_daemon(self, config: ManagerConfig) -> CliResult<()> {
        use crate::cli::Spawner;
        let pid = Spawner::spawn_running_background(Vec::new())?;
        println!("[distant manager detached, pid = {}]", pid);
        Ok(())
    }

    #[cfg(unix)]
    fn run_daemon(self, config: ManagerConfig) -> CliResult<()> {
        use fork::{daemon, Fork};

        debug!("Forking process");
        match daemon(true, true) {
            Ok(Fork::Child) => {
                let rt = tokio::runtime::Runtime::new()?;
                rt.block_on(async { Self::async_run(self, config).await })?;
                Ok(())
            }
            Ok(Fork::Parent(pid)) => {
                println!("[distant manager detached, pid = {}]", pid);
                if fork::close_fd().is_err() {
                    Err(io::Error::new(io::ErrorKind::Other, "Fork failed to close fd").into())
                } else {
                    Ok(())
                }
            }
            Err(_) => Err(io::Error::new(io::ErrorKind::Other, "Fork failed").into()),
        }
    }

    async fn async_run(self, config: ManagerConfig) -> CliResult<()> {
        match self {
            Self::Service(ManagerServiceSubcommand::Start { kind }) => {
                debug!("Starting manager service via {:?}", kind);
                let service = <dyn Service>::target_or_native(kind)?;
                service.start(ServiceStartCtx {
                    label: SERVICE_LABEL.clone(),
                })?;
                Ok(())
            }
            Self::Service(ManagerServiceSubcommand::Stop { kind }) => {
                debug!("Stopping manager service via {:?}", kind);
                let service = <dyn Service>::target_or_native(kind)?;
                service.stop(ServiceStopCtx {
                    label: SERVICE_LABEL.clone(),
                })?;
                Ok(())
            }
            Self::Service(ManagerServiceSubcommand::Install { kind, user }) => {
                debug!("Installing manager service via {:?}", kind);
                let service = <dyn Service>::target_or_native(kind)?;
                service.install(ServiceInstallCtx {
                    label: SERVICE_LABEL.clone(),
                    user,

                    // distant manager listen
                    program: std::env::current_exe()
                        .ok()
                        .and_then(|p| p.to_str().map(ToString::to_string))
                        .unwrap_or_else(|| String::from("distant")),
                    args: vec!["manager".to_string(), "listen".to_string()],
                })?;

                // TODO: The cleanest way I can think of to support user-level installation
                //       for platforms that support it (launchd, systemd) is to generate or
                //       modify a user-level config file such that the client and manager
                //       point to a user-specific socket like "/run/user/1001/distant.sock"
                //       instead of "/run/distant.sock" or pipe name like "{user}.distant"
                //       instead of "distant". That way, root-level managers will be accessed
                //       by clients by default and only users that have configured user-level
                //       managers will automatically connect to them
                todo!("Generate or update config.toml at user level with custom socket/pipe");

                Ok(())
            }
            Self::Service(ManagerServiceSubcommand::Uninstall { kind, user }) => {
                debug!("Uninstalling manager service via {:?}", kind);
                let service = <dyn Service>::target_or_native(kind)?;
                service.uninstall(ServiceUninstallCtx {
                    label: SERVICE_LABEL.clone(),
                    user,
                })?;

                // TODO: It's unclear what to do here other than load up a user-level config
                //       file if it exists and either remove or reset the socket and pipe
                //       configuration such that it points to the global socket/pipe instead
                todo!("Remove or reset socket/pipe configuration");

                Ok(())
            }
            Self::Listen { .. } => {
                debug!("Starting manager: {:?}", config.network.as_os_str());
                let manager_ref = Manager::new(DistantManagerConfig::default(), config.network)
                    .listen()
                    .await?;

                // Register our handlers for different schemes
                debug!("Registering handlers with manager");
                manager_ref
                    .register_launch_handler("manager", handlers::ManagerLaunchHandler)
                    .await?;
                manager_ref
                    .register_launch_handler("ssh", handlers::SshLaunchHandler)
                    .await?;
                manager_ref
                    .register_connect_handler("distant", handlers::DistantConnectHandler)
                    .await?;
                manager_ref
                    .register_connect_handler("ssh", handlers::SshConnectHandler)
                    .await?;

                // Let our server run to completion
                manager_ref.wait().await?;
                debug!("Manager is shutting down");

                Ok(())
            }
            Self::Info { id } => {
                debug!(
                    "Getting info about connection {} from manager: {:?}",
                    id,
                    config.network.as_os_str()
                );
                let info = Client::new(config.network)
                    .connect()
                    .await?
                    .info(id)
                    .await?;

                #[derive(Tabled)]
                struct InfoRow {
                    id: ConnectionId,
                    scheme: String,
                    host: String,
                    port: String,
                    extra: String,
                }

                println!(
                    "{}",
                    Table::new(vec![InfoRow {
                        id: info.id,
                        scheme: info
                            .destination
                            .scheme()
                            .map(ToString::to_string)
                            .unwrap_or_default(),
                        host: info.destination.to_host_string(),
                        port: info
                            .destination
                            .port()
                            .map(|x| x.to_string())
                            .unwrap_or_default(),
                        extra: info.extra.to_string()
                    }])
                );

                Ok(())
            }
            Self::List => {
                debug!(
                    "Getting list of connections from manager: {:?}",
                    config.network.as_os_str()
                );
                let list = Client::new(config.network).connect().await?.list().await?;

                #[derive(Tabled)]
                struct ListRow {
                    id: ConnectionId,
                    scheme: String,
                    host: String,
                    port: String,
                }

                println!(
                    "{}",
                    Table::new(list.into_iter().map(|(id, destination)| {
                        ListRow {
                            id,
                            scheme: destination
                                .scheme()
                                .map(ToString::to_string)
                                .unwrap_or_default(),
                            host: destination.to_host_string(),
                            port: destination
                                .port()
                                .map(|x| x.to_string())
                                .unwrap_or_default(),
                        }
                    }))
                );

                Ok(())
            }
            Self::Kill { id } => {
                debug!(
                    "Killing connection {} from manager: {:?}",
                    id,
                    config.network.as_os_str()
                );
                Client::new(config.network)
                    .connect()
                    .await?
                    .kill(id)
                    .await?;
                Ok(())
            }
            Self::Shutdown => {
                debug!("Shutting down manager: {:?}", config.network.as_os_str());
                Client::new(config.network)
                    .connect()
                    .await?
                    .shutdown()
                    .await?;
                Ok(())
            }
        }
    }
}
