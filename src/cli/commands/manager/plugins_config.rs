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

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;

    // -------------------------------------------------------
    // PluginsConfig deserialization
    // -------------------------------------------------------
    #[test]
    fn plugins_config_empty_toml() {
        let config: PluginsConfig = toml_edit::de::from_str("").unwrap();
        assert!(config.plugins.is_empty());
    }

    #[test]
    fn plugins_config_with_one_plugin() {
        let toml = r#"
[plugins.docker]
path = "/usr/local/bin/distant-plugin-docker"
"#;
        let config: PluginsConfig = toml_edit::de::from_str(toml).unwrap();
        assert_eq!(config.plugins.len(), 1);
        let entry = config.plugins.get("docker").unwrap();
        assert_eq!(
            entry.path,
            PathBuf::from("/usr/local/bin/distant-plugin-docker")
        );
        assert!(entry.schemes.is_none());
    }

    #[test]
    fn plugins_config_with_schemes() {
        let toml = r#"
[plugins.docker]
path = "/usr/local/bin/distant-plugin-docker"
schemes = ["docker", "container"]
"#;
        let config: PluginsConfig = toml_edit::de::from_str(toml).unwrap();
        let entry = config.plugins.get("docker").unwrap();
        assert_eq!(
            entry.schemes.as_ref().unwrap(),
            &vec!["docker".to_string(), "container".to_string()]
        );
    }

    #[test]
    fn plugins_config_with_multiple_plugins() {
        let toml = r#"
[plugins.docker]
path = "/usr/local/bin/docker-plugin"

[plugins.k8s]
path = "/usr/local/bin/k8s-plugin"
schemes = ["kubernetes"]
"#;
        let config: PluginsConfig = toml_edit::de::from_str(toml).unwrap();
        assert_eq!(config.plugins.len(), 2);
        assert!(config.plugins.contains_key("docker"));
        assert!(config.plugins.contains_key("k8s"));
    }

    // -------------------------------------------------------
    // load_external_plugins with CLI-only plugins
    // -------------------------------------------------------
    #[test]
    fn load_external_plugins_with_cli_plugins() {
        let cli_plugins = vec![
            (
                "docker".to_string(),
                PathBuf::from("/usr/local/bin/docker-plugin"),
            ),
            (
                "k8s".to_string(),
                PathBuf::from("/usr/local/bin/k8s-plugin"),
            ),
        ];
        let plugins = load_external_plugins(cli_plugins).unwrap();
        assert_eq!(plugins.len(), 2);
    }

    #[test]
    fn load_external_plugins_with_empty_cli_plugins() {
        let plugins = load_external_plugins(Vec::new()).unwrap();
        // May or may not have plugins from config file
        // We just check it doesn't fail
        let _ = plugins;
    }

    // -------------------------------------------------------
    // dirs_config_path
    // -------------------------------------------------------
    #[test]
    fn dirs_config_path_returns_some_on_most_systems() {
        // On most systems with a home dir, this should return Some
        let path = dirs_config_path();
        // We just check it doesn't panic
        let _ = path;
    }
}
