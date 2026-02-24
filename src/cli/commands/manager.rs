use std::collections::HashMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use distant_core::Plugin;
use distant_core::net::manager::Config as NetManagerConfig;
use log::*;
use once_cell::sync::Lazy;
use service_manager::{
    ServiceInstallCtx, ServiceLabel, ServiceLevel, ServiceManager, ServiceStartCtx, ServiceStopCtx,
    ServiceUninstallCtx,
};

#[cfg(unix)]
use crate::CliError;
use crate::CliResult;
use crate::cli::Manager;
use crate::cli::common::{Ui, connect_to_manager};
use crate::options::{Format, ManagerServiceSubcommand, ManagerSubcommand};

/// [`ServiceLabel`] for our manager in the form `rocks.distant.manager`
static SERVICE_LABEL: Lazy<ServiceLabel> = Lazy::new(|| ServiceLabel {
    qualifier: String::from("rocks"),
    organization: String::from("distant"),
    application: String::from("manager"),
});

mod handlers;
mod plugins_config;

/// Collect all plugins (built-in + external from config) and register them by scheme.
/// Returns an error if two plugins claim the same scheme.
fn build_plugin_map(
    extra_plugins: Vec<(String, PathBuf)>,
) -> anyhow::Result<HashMap<String, Arc<dyn Plugin>>> {
    let mut map: HashMap<String, Arc<dyn Plugin>> = HashMap::new();

    // Built-in plugins
    let builtins: Vec<Arc<dyn Plugin>> = vec![
        Arc::new(handlers::DistantPlugin::new()),
        Arc::new(handlers::SshPlugin),
    ];

    // External plugins from config file + CLI flags
    let external = plugins_config::load_external_plugins(extra_plugins)?;

    let all_plugins = builtins.into_iter().chain(external);

    for plugin in all_plugins {
        for scheme in plugin.schemes() {
            let scheme = scheme.to_lowercase();
            if let Some(existing) = map.get(&scheme) {
                anyhow::bail!(
                    "Scheme '{}' is already registered by plugin '{}', \
                     cannot also register it for plugin '{}'",
                    scheme,
                    existing.name(),
                    plugin.name()
                );
            }
            map.insert(scheme, Arc::clone(&plugin));
        }
    }

    Ok(map)
}

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
    use fork::{Fork, daemon};

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
            plugin: extra_plugins,
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

            let plugins = build_plugin_map(extra_plugins).context("Failed to register plugins")?;

            let manager = Manager {
                #[cfg(unix)]
                access,
                config: NetManagerConfig {
                    user,
                    plugins,
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
            // Always use Shell auth (prompt-based) for the connection — the format
            // flag only controls output formatting, not the auth protocol.
            let mut client = connect_to_manager(Format::Shell, network, &ui).await?;

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

#[cfg(test)]
mod tests {
    //! Tests for `SERVICE_LABEL` constants and `build_plugin_map` — verifying
    //! built-in plugin registration, scheme naming, and duplicate rejection.

    use test_log::test;

    use super::*;

    // -------------------------------------------------------
    // SERVICE_LABEL
    // -------------------------------------------------------
    #[test]
    fn service_label_has_correct_fields() {
        assert_eq!(SERVICE_LABEL.qualifier, "rocks");
        assert_eq!(SERVICE_LABEL.organization, "distant");
        assert_eq!(SERVICE_LABEL.application, "manager");
    }

    // -------------------------------------------------------
    // build_plugin_map — no extra plugins
    // -------------------------------------------------------
    #[test]
    fn build_plugin_map_with_no_extras_has_builtins() {
        let map = build_plugin_map(Vec::new()).unwrap();
        // Should contain "distant" and "ssh"
        assert!(map.contains_key("distant"), "missing 'distant' scheme");
        assert!(map.contains_key("ssh"), "missing 'ssh' scheme");
    }

    #[test]
    fn build_plugin_map_builtins_have_correct_names() {
        let map = build_plugin_map(Vec::new()).unwrap();
        assert_eq!(map["distant"].name(), "distant");
        assert_eq!(map["ssh"].name(), "ssh");
    }

    // -------------------------------------------------------
    // build_plugin_map — schemes are lowercased
    // -------------------------------------------------------
    #[test]
    fn build_plugin_map_schemes_are_lowercase() {
        let map = build_plugin_map(Vec::new()).unwrap();
        for key in map.keys() {
            assert_eq!(key, &key.to_lowercase(), "scheme should be lowercase");
        }
    }

    // -------------------------------------------------------
    // build_plugin_map — with extra CLI plugins
    // -------------------------------------------------------
    #[test]
    fn build_plugin_map_with_extra_plugins() {
        let extras = vec![(
            "myplugin".to_string(),
            PathBuf::from("/usr/local/bin/myplugin"),
        )];
        let map = build_plugin_map(extras).unwrap();
        // Should contain builtins plus the extra
        assert!(map.contains_key("distant"));
        assert!(map.contains_key("ssh"));
        assert!(map.contains_key("myplugin"), "missing 'myplugin' scheme");
    }

    // -------------------------------------------------------
    // build_plugin_map — duplicate scheme detection
    // -------------------------------------------------------
    #[test]
    fn build_plugin_map_rejects_duplicate_builtin_scheme() {
        // Trying to register a plugin with scheme "ssh" should fail
        // because SSH is already a builtin
        let extras = vec![("ssh".to_string(), PathBuf::from("/usr/local/bin/other-ssh"))];
        let result = build_plugin_map(extras);
        assert!(result.is_err(), "should reject duplicate 'ssh' scheme");
        let err = result.err().unwrap().to_string();
        assert!(
            err.contains("ssh") && err.contains("already registered"),
            "error should mention scheme conflict: {err}"
        );
    }

    // -------------------------------------------------------
    // build_plugin_map — multiple extra plugins
    // -------------------------------------------------------
    #[test]
    fn build_plugin_map_with_multiple_extra_plugins() {
        let extras = vec![
            ("docker".to_string(), PathBuf::from("/bin/docker-plugin")),
            ("k8s".to_string(), PathBuf::from("/bin/k8s-plugin")),
        ];
        let map = build_plugin_map(extras).unwrap();
        assert!(map.contains_key("docker"));
        assert!(map.contains_key("k8s"));
        assert!(map.contains_key("distant"));
        assert!(map.contains_key("ssh"));
    }
}
