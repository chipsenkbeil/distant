use super::common;
use crate::constants;
use anyhow::Context;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

mod client;
mod generate;
mod manager;
mod server;

pub use client::*;
pub use generate::*;
pub use manager::*;
pub use server::*;

const DEFAULT_RAW_STR: &str = include_str!("config.toml");

/// Represents configuration settings for all of distant
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub client: ClientConfig,
    pub generate: GenerateConfig,
    pub manager: ManagerConfig,
    pub server: ServerConfig,
}

impl Config {
    /// Returns a reference to the default config file as a raw str.
    pub const fn default_raw_str() -> &'static str {
        DEFAULT_RAW_STR
    }

    /// Loads the configuration from multiple sources in a blocking fashion
    ///
    /// 1. If `custom` is provided, it is used by itself as the source for configuration
    /// 2. Otherwise, if `custom` is not provided, will attempt to load from global and user
    ///    config files, merging together if they both exist
    /// 3. Otherwise if no `custom` path and none of the standard configuration paths exist,
    ///    then the default configuration is returned instead
    pub fn load_multi(custom: Option<PathBuf>) -> anyhow::Result<Self> {
        match custom {
            Some(path) => {
                toml_edit::de::from_slice(&std::fs::read(path)?).context("Failed to parse config")
            }
            None => {
                let paths = vec![
                    constants::global::CONFIG_FILE_PATH.as_path(),
                    constants::user::CONFIG_FILE_PATH.as_path(),
                ];

                match (paths[0].exists(), paths[1].exists()) {
                    // At least one standard path exists, so load it
                    (exists_1, exists_2) if exists_1 || exists_2 => {
                        use config::{Config, File};
                        let config = Config::builder()
                            .add_source(File::from(paths[0]).required(exists_1))
                            .add_source(File::from(paths[1]).required(exists_2))
                            .build()
                            .context("Failed to build config from paths")?;
                        config.try_deserialize().context("Failed to parse config")
                    }

                    // None of our standard paths exist, so use the default value instead
                    _ => Ok(Self::default()),
                }
            }
        }
    }

    /// Loads the specified `path` as a [`Config`]
    #[cfg(test)]
    pub async fn load(path: impl AsRef<std::path::Path>) -> anyhow::Result<Self> {
        let bytes = tokio::fs::read(path.as_ref())
            .await
            .with_context(|| format!("Failed to read config file {:?}", path.as_ref()))?;
        toml_edit::de::from_slice(&bytes).context("Failed to parse config")
    }
}

impl Default for Config {
    fn default() -> Self {
        static DEFAULT_CONFIG: Lazy<Config> = Lazy::new(|| {
            toml_edit::de::from_str(Config::default_raw_str())
                .expect("Default config failed to parse")
        });

        DEFAULT_CONFIG.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use common::*;
    use distant_core::net::common::{Host, Map, PortRange};
    use distant_core::net::map;
    use distant_core::net::server::Shutdown;
    use std::net::Ipv4Addr;
    use std::time::Duration;
    use test_log::test;

    #[test]
    fn default_should_parse_config_from_internal_toml() {
        let config = Config::default();
        assert_eq!(
            config,
            Config {
                client: ClientConfig {
                    action: ClientActionConfig { timeout: Some(0.) },
                    connect: ClientConnectConfig {
                        options: Map::new()
                    },
                    launch: ClientLaunchConfig {
                        distant: ClientLaunchDistantConfig {
                            bin: Some("distant".to_owned()),
                            bind_server: Some(BindAddress::Ssh),
                            args: Some("".to_string())
                        },
                        options: Map::new(),
                    },
                    logging: LoggingSettings {
                        log_level: Some(LogLevel::Info),
                        log_file: None
                    },
                    network: NetworkSettings {
                        unix_socket: None,
                        windows_pipe: None
                    },
                    repl: ClientReplConfig { timeout: Some(0.) },
                },
                generate: GenerateConfig {
                    logging: LoggingSettings {
                        log_level: Some(LogLevel::Info),
                        log_file: None
                    },
                },
                manager: ManagerConfig {
                    access: Some(AccessControl::Owner),
                    logging: LoggingSettings {
                        log_level: Some(LogLevel::Info),
                        log_file: None
                    },
                    network: NetworkSettings {
                        unix_socket: None,
                        windows_pipe: None
                    },
                },
                server: ServerConfig {
                    listen: ServerListenConfig {
                        host: Some(BindAddress::Any),
                        port: Some(0.into()),
                        use_ipv6: false,
                        shutdown: Some(Shutdown::Never),
                        current_dir: None,
                    },
                    logging: LoggingSettings {
                        log_level: Some(LogLevel::Info),
                        log_file: None
                    },
                },
            }
        );
    }

    #[test(tokio::test)]
    async fn load_should_parse_config_from_specified_file() {
        use assert_fs::prelude::*;
        let config_file = assert_fs::NamedTempFile::new("config.toml").unwrap();
        config_file
            .write_str(
                r#"
[client]
log_file = "client-log-file"
log_level = "trace"
unix_socket = "client-unix-socket"
windows_pipe = "client-windows-pipe"

[client.action]
timeout = 123

[client.connect]
options = "key=\"value\",key2=\"value2\""

[client.launch]
bin = "some-bin"
bind_server = "any"
args = "a b c"
options = "key3=\"value3\",key4=\"value4\""

[client.repl]
timeout = 456

[generate]
log_file = "generate-log-file"
log_level = "debug"

[manager]
log_file = "manager-log-file"
log_level = "warn"
access = "anyone"
unix_socket = "manager-unix-socket"
windows_pipe = "manager-windows-pipe"

[server]
log_file = "server-log-file"
log_level = "error"

[server.listen]
host = "127.0.0.1"
port = "8080:8089"
use_ipv6 = true
shutdown = "after=123"
current_dir = "server-current-dir"
"#,
            )
            .unwrap();

        let config = Config::load(config_file.path()).await.unwrap();
        assert_eq!(
            config,
            Config {
                client: ClientConfig {
                    action: ClientActionConfig {
                        timeout: Some(123.)
                    },
                    connect: ClientConnectConfig {
                        options: map!("key" -> "value", "key2" -> "value2"),
                    },
                    launch: ClientLaunchConfig {
                        distant: ClientLaunchDistantConfig {
                            bin: Some("some-bin".to_owned()),
                            bind_server: Some(BindAddress::Any),
                            args: Some(String::from("a b c"))
                        },
                        options: map!("key3" -> "value3", "key4" -> "value4"),
                    },
                    logging: LoggingSettings {
                        log_level: Some(LogLevel::Trace),
                        log_file: Some(PathBuf::from("client-log-file")),
                    },
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("client-unix-socket")),
                        windows_pipe: Some(String::from("client-windows-pipe"))
                    },
                    repl: ClientReplConfig {
                        timeout: Some(456.)
                    },
                },
                generate: GenerateConfig {
                    logging: LoggingSettings {
                        log_level: Some(LogLevel::Debug),
                        log_file: Some(PathBuf::from("generate-log-file"))
                    },
                },
                manager: ManagerConfig {
                    access: Some(AccessControl::Anyone),
                    logging: LoggingSettings {
                        log_level: Some(LogLevel::Warn),
                        log_file: Some(PathBuf::from("manager-log-file"))
                    },
                    network: NetworkSettings {
                        unix_socket: Some(PathBuf::from("manager-unix-socket")),
                        windows_pipe: Some(String::from("manager-windows-pipe")),
                    },
                },
                server: ServerConfig {
                    listen: ServerListenConfig {
                        host: Some(BindAddress::Host(Host::Ipv4(Ipv4Addr::new(127, 0, 0, 1)))),
                        port: Some(PortRange {
                            start: 8080,
                            end: Some(8089)
                        }),
                        use_ipv6: true,
                        shutdown: Some(Shutdown::After(Duration::from_secs(123))),
                        current_dir: Some(PathBuf::from("server-current-dir")),
                    },
                    logging: LoggingSettings {
                        log_level: Some(LogLevel::Error),
                        log_file: Some(PathBuf::from("server-log-file")),
                    },
                },
            }
        );
    }
}
