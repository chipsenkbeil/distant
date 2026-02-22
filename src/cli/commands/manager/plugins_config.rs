use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use distant_core::plugin::ProcessPlugin;
use distant_core::Plugin;
use log::*;
use serde::Deserialize;

/// Top-level structure for `~/.config/distant/plugins.toml`.
#[derive(Deserialize, Default)]
struct PluginsConfig {
    #[serde(default)]
    plugins: HashMap<String, PluginEntry>,
}

/// A single plugin entry in the TOML config.
#[derive(Deserialize)]
struct PluginEntry {
    path: PathBuf,
    schemes: Option<Vec<String>>,
}

/// Load external plugins from `~/.config/distant/plugins.toml` (if it exists)
/// and any extra `--plugin NAME=PATH` pairs from the CLI.
pub fn load_external_plugins(
    cli_plugins: Vec<(String, PathBuf)>,
) -> anyhow::Result<Vec<Arc<dyn Plugin>>> {
    let mut plugins: Vec<Arc<dyn Plugin>> = Vec::new();

    // Load from config file
    if let Some(config_dir) = dirs_config_path() {
        let config_path = config_dir.join("plugins.toml");
        if config_path.exists() {
            debug!("Loading plugins config from {}", config_path.display());
            let contents = std::fs::read_to_string(&config_path)?;
            let config: PluginsConfig = toml_edit::de::from_str(&contents)?;

            for (name, entry) in config.plugins {
                debug!(
                    "Loaded external plugin '{}' from config: {}",
                    name,
                    entry.path.display()
                );
                plugins.push(Arc::new(ProcessPlugin {
                    name,
                    path: entry.path,
                    schemes: entry.schemes,
                }));
            }
        }
    }

    // Load from CLI --plugin flags
    for (name, path) in cli_plugins {
        debug!(
            "Loaded external plugin '{}' from CLI: {}",
            name,
            path.display()
        );
        plugins.push(Arc::new(ProcessPlugin {
            name,
            path,
            schemes: None,
        }));
    }

    Ok(plugins)
}

fn dirs_config_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "distant").map(|dirs| dirs.config_dir().to_path_buf())
}
