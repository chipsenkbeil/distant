use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use distant_core::Plugin;
use distant_core::net::manager::Config as NetManagerConfig;
use log::*;

#[cfg(unix)]
use crate::CliError;
use crate::CliResult;
use crate::cli::Manager;
use crate::cli::common::{Ui, connect_to_manager};
use crate::options::{Format, ManagerSubcommand};

mod plugins;

/// Collect all plugins (built-in + external from config) and register them by scheme.
/// Returns an error if two plugins claim the same scheme.
#[allow(clippy::vec_init_then_push)]
/// Builds the mount plugin map from enabled features.
fn build_mount_plugin_map() -> HashMap<String, Arc<dyn distant_core::plugin::MountPlugin>> {
    let mut map: HashMap<String, Arc<dyn distant_core::plugin::MountPlugin>> = HashMap::new();

    #[cfg(feature = "mount-nfs")]
    map.insert(
        "nfs".into(),
        Arc::new(distant_mount::plugin::NfsMountPlugin),
    );

    #[cfg(all(
        feature = "mount-fuse",
        any(target_os = "linux", target_os = "freebsd", target_os = "macos")
    ))]
    map.insert(
        "fuse".into(),
        Arc::new(distant_mount::plugin::FuseMountPlugin),
    );

    #[cfg(all(feature = "mount-macos-file-provider", target_os = "macos"))]
    map.insert(
        "macos-file-provider".into(),
        Arc::new(distant_mount::plugin::FileProviderMountPlugin),
    );

    #[cfg(all(feature = "mount-windows-cloud-files", target_os = "windows"))]
    map.insert(
        "windows-cloud-files".into(),
        Arc::new(distant_mount::plugin::CloudFilesMountPlugin),
    );

    map
}

fn build_plugin_map(
    extra_plugins: Vec<(String, PathBuf)>,
) -> anyhow::Result<HashMap<String, Arc<dyn Plugin>>> {
    let mut map: HashMap<String, Arc<dyn Plugin>> = HashMap::new();

    // Built-in plugins — conditionally populated based on enabled features
    let builtins: Vec<Arc<dyn Plugin>> = vec![
        #[cfg(feature = "host")]
        Arc::new(distant_host::HostPlugin::new()),
        #[cfg(feature = "ssh")]
        Arc::new(distant_ssh::SshPlugin),
        #[cfg(feature = "docker")]
        Arc::new(distant_docker::DockerPlugin),
    ];

    // External plugins from config file + CLI flags
    let external = plugins::load_external_plugins(extra_plugins)?;

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

pub fn run(cmd: ManagerSubcommand, quiet: bool) -> CliResult {
    match &cmd {
        ManagerSubcommand::Listen { daemon, .. } if *daemon => run_daemon(cmd, quiet),
        _ => {
            let rt = tokio::runtime::Runtime::new().context("Failed to start up runtime")?;
            rt.block_on(async_run(cmd, quiet))
        }
    }
}

#[cfg(windows)]
fn run_daemon(_cmd: ManagerSubcommand, _quiet: bool) -> CliResult {
    use crate::cli::Spawner;
    let pid = Spawner::spawn_running_background(Vec::new())
        .context("Failed to spawn background process")?;
    println!("[distant manager detached, pid = {}]", pid);
    Ok(())
}

#[cfg(unix)]
fn run_daemon(cmd: ManagerSubcommand, quiet: bool) -> CliResult {
    use fork::{Fork, daemon};

    debug!("Forking process");
    match daemon(true, true) {
        Ok(Fork::Child) => {
            let rt = tokio::runtime::Runtime::new().context("Failed to start up runtime")?;
            rt.block_on(async { async_run(cmd, quiet).await })?;
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

async fn async_run(cmd: ManagerSubcommand, quiet: bool) -> CliResult {
    let ui = Ui::new(quiet);

    match cmd {
        ManagerSubcommand::Listen {
            #[cfg_attr(not(unix), allow(unused_variables))]
            access,
            daemon: _daemon,
            network,
            user,
            shutdown,
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
            let mount_plugins = build_mount_plugin_map();

            let manager = Manager {
                #[cfg(unix)]
                access,
                config: NetManagerConfig {
                    user,
                    plugins,
                    mount_plugins,
                    ..Default::default()
                },
                shutdown: shutdown.into_inner(),
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
    //! Tests for `build_plugin_map` — verifying built-in plugin registration,
    //! scheme naming, and duplicate rejection.

    use test_log::test;

    use super::*;

    // -------------------------------------------------------
    // build_plugin_map — no extra plugins
    // -------------------------------------------------------
    #[test]
    fn build_plugin_map_with_no_extras_has_builtins() {
        let map = build_plugin_map(Vec::new()).unwrap();
        #[cfg(feature = "host")]
        assert!(map.contains_key("distant"), "missing 'distant' scheme");
        #[cfg(feature = "ssh")]
        assert!(map.contains_key("ssh"), "missing 'ssh' scheme");
        let _ = &map; // suppress unused warning when no features enabled
    }

    #[cfg(all(feature = "host", feature = "ssh"))]
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
        #[cfg(feature = "host")]
        assert!(map.contains_key("distant"));
        #[cfg(feature = "ssh")]
        assert!(map.contains_key("ssh"));
        assert!(map.contains_key("myplugin"), "missing 'myplugin' scheme");
    }

    // -------------------------------------------------------
    // build_plugin_map — duplicate scheme detection
    // -------------------------------------------------------
    #[cfg(feature = "ssh")]
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
            ("custom".to_string(), PathBuf::from("/bin/custom-plugin")),
            ("k8s".to_string(), PathBuf::from("/bin/k8s-plugin")),
        ];
        let map = build_plugin_map(extras).unwrap();
        assert!(map.contains_key("custom"));
        assert!(map.contains_key("k8s"));
        #[cfg(feature = "host")]
        assert!(map.contains_key("distant"));
        #[cfg(feature = "ssh")]
        assert!(map.contains_key("ssh"));
    }
}
