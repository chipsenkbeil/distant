//! SSH configuration resolution.
//!
//! Provides [`ResolvedConfig`] as a unified representation of SSH configuration
//! from either `ssh -G` output or the built-in ssh2-config parser.

use std::fs::File;
use std::io::{self, BufReader};
use std::path::{Path, PathBuf};
use std::time::Duration;

use log::*;
use ssh2_config::{HostParams, ParseRule, SshConfig};

use crate::SshOpts;

/// SSH timeout for `ssh -G` queries, in seconds.
const SSH_G_TIMEOUT_SECS: u64 = 10;

/// Resolved SSH configuration from either `ssh -G` or the built-in parser.
///
/// Serves as an intermediate representation between raw SSH config sources
/// and the russh client configuration. Algorithm fields are plain `Vec<String>`,
/// avoiding the need for `AlgorithmsRule` from ssh2-config.
#[derive(Clone, Debug, Default)]
pub(crate) struct ResolvedConfig {
    pub host_name: Option<String>,
    pub port: Option<u16>,
    pub user: Option<String>,
    pub identity_files: Vec<PathBuf>,
    pub proxy_command: Option<String>,
    pub proxy_jump: Option<Vec<String>>,
    pub server_alive_interval: Option<Duration>,
    pub tcp_keep_alive: Option<bool>,
    pub connect_timeout: Option<Duration>,
    pub compression: Option<bool>,
    pub ciphers: Vec<String>,
    pub kex_algorithms: Vec<String>,
    pub host_key_algorithms: Vec<String>,
    pub macs: Vec<String>,
    pub pubkey_accepted_algorithms: Vec<String>,
    pub strict_host_key_checking: Option<String>,
    pub identities_only: Option<String>,
    pub identity_agent: Option<String>,
    pub user_known_hosts_files: Vec<String>,
    pub global_known_hosts_files: Vec<String>,
}

impl ResolvedConfig {
    /// Convert a [`HostParams`] from the ssh2-config parser into a [`ResolvedConfig`].
    fn from_host_params(params: &HostParams) -> Self {
        Self {
            host_name: params.host_name.clone(),
            port: params.port,
            user: params.user.clone(),
            identity_files: params.identity_file.clone().unwrap_or_default(),
            proxy_command: params
                .unsupported_fields
                .get("proxycommand")
                .map(|v| v.join(" "))
                .filter(|s| !s.is_empty()),
            proxy_jump: params.proxy_jump.clone(),
            server_alive_interval: params.server_alive_interval,
            tcp_keep_alive: params.tcp_keep_alive,
            connect_timeout: params.connect_timeout,
            compression: params.compression,
            ciphers: if params.ciphers.is_default() {
                Vec::new()
            } else {
                params.ciphers.algorithms().to_vec()
            },
            kex_algorithms: if params.kex_algorithms.is_default() {
                Vec::new()
            } else {
                params.kex_algorithms.algorithms().to_vec()
            },
            host_key_algorithms: if params.host_key_algorithms.is_default() {
                Vec::new()
            } else {
                params.host_key_algorithms.algorithms().to_vec()
            },
            macs: if params.mac.is_default() {
                Vec::new()
            } else {
                params.mac.algorithms().to_vec()
            },
            pubkey_accepted_algorithms: if params.pubkey_accepted_algorithms.is_default() {
                Vec::new()
            } else {
                params.pubkey_accepted_algorithms.algorithms().to_vec()
            },
            strict_host_key_checking: params
                .unsupported_fields
                .get("stricthostkeychecking")
                .and_then(|v| v.first())
                .cloned(),
            identities_only: params
                .unsupported_fields
                .get("identitiesonly")
                .and_then(|v| v.first())
                .cloned(),
            identity_agent: params
                .unsupported_fields
                .get("identityagent")
                .and_then(|v| v.first())
                .cloned(),
            user_known_hosts_files: params
                .unsupported_fields
                .get("userknownhostsfile")
                .cloned()
                .unwrap_or_default(),
            global_known_hosts_files: params
                .unsupported_fields
                .get("globalknownhostsfile")
                .cloned()
                .unwrap_or_default(),
        }
    }
}

/// Query OpenSSH for resolved config via `ssh -G hostname`.
///
/// This delegates to the system's `ssh` binary, which handles `Match` blocks,
/// `Include` directives, and other advanced config features that the built-in
/// ssh2-config parser does not support.
///
/// Returns `None` if `ssh` is not available or the command fails.
pub(crate) async fn query_openssh(host: &str) -> Option<ResolvedConfig> {
    use tokio::process::Command;

    let output = match tokio::time::timeout(
        Duration::from_secs(SSH_G_TIMEOUT_SECS),
        Command::new("ssh")
            .args(["-G", "--", host])
            .stdin(std::process::Stdio::null())
            .output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            debug!("ssh -G failed to execute: {e}");
            return None;
        }
        Err(_) => {
            warn!("ssh -G timed out after {SSH_G_TIMEOUT_SECS}s");
            return None;
        }
    };

    if !output.status.success() {
        debug!("ssh -G exited with status {}", output.status);
        return None;
    }

    match String::from_utf8(output.stdout) {
        Ok(stdout) => parse_ssh_g_output(&stdout),
        Err(e) => {
            debug!("ssh -G output was not valid UTF-8: {e}");
            None
        }
    }
}

/// Parse the flat key-value output of `ssh -G` into a [`ResolvedConfig`].
///
/// The `ssh -G` command prints resolved configuration as lowercase
/// `key value` pairs, one per line. This method maps recognized keys
/// into the corresponding [`ResolvedConfig`] fields.
pub(crate) fn parse_ssh_g_output(output: &str) -> Option<ResolvedConfig> {
    let mut config = ResolvedConfig::default();
    let mut identity_files: Vec<PathBuf> = Vec::new();
    let mut has_content = false;

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        has_content = true;

        // ssh -G output is "key value" (space-separated, lowercase keys)
        let (key, value) = match line.split_once(' ') {
            Some((k, v)) => (k, v.trim()),
            None => continue,
        };

        match key {
            "hostname" => {
                config.host_name = Some(value.to_string());
            }
            "port" => {
                if let Ok(p) = value.parse::<u16>() {
                    config.port = Some(p);
                }
            }
            "user" => {
                config.user = Some(value.to_string());
            }
            "identityfile" => {
                identity_files.push(PathBuf::from(value));
            }
            "proxycommand" => {
                if !value.eq_ignore_ascii_case("none") {
                    config.proxy_command = Some(value.to_string());
                }
            }
            "proxyjump" => {
                if !value.eq_ignore_ascii_case("none") {
                    config.proxy_jump =
                        Some(value.split(',').map(|s| s.trim().to_string()).collect());
                }
            }
            "identitiesonly" => {
                config.identities_only = Some(value.to_string());
            }
            "userknownhostsfile" => {
                config.user_known_hosts_files =
                    value.split_whitespace().map(|s| s.to_string()).collect();
            }
            "globalknownhostsfile" => {
                config.global_known_hosts_files =
                    value.split_whitespace().map(|s| s.to_string()).collect();
            }
            "stricthostkeychecking" => {
                config.strict_host_key_checking = Some(value.to_string());
            }
            "serveraliveinterval" => {
                if let Ok(secs) = value.parse::<u64>()
                    && secs > 0
                {
                    config.server_alive_interval = Some(Duration::from_secs(secs));
                }
            }
            "tcpkeepalive" => {
                config.tcp_keep_alive = Some(value.eq_ignore_ascii_case("yes"));
            }
            "connecttimeout" => {
                if let Ok(secs) = value.parse::<u64>()
                    && secs > 0
                {
                    config.connect_timeout = Some(Duration::from_secs(secs));
                }
            }
            "compression" => {
                config.compression = Some(value.eq_ignore_ascii_case("yes"));
            }
            "ciphers" => {
                config.ciphers = value.split(',').map(|s| s.trim().to_string()).collect();
            }
            "kexalgorithms" => {
                config.kex_algorithms = value.split(',').map(|s| s.trim().to_string()).collect();
            }
            "hostkeyalgorithms" => {
                config.host_key_algorithms =
                    value.split(',').map(|s| s.trim().to_string()).collect();
            }
            "macs" => {
                config.macs = value.split(',').map(|s| s.trim().to_string()).collect();
            }
            "pubkeyacceptedalgorithms" => {
                config.pubkey_accepted_algorithms =
                    value.split(',').map(|s| s.trim().to_string()).collect();
            }
            "identityagent" => {
                if !value.eq_ignore_ascii_case("none") {
                    config.identity_agent = Some(value.to_string());
                }
            }
            _ => {
                // Ignore other fields
            }
        }
    }

    if !has_content {
        return None;
    }

    if !identity_files.is_empty() {
        config.identity_files = identity_files;
    }

    Some(config)
}

/// Parse SSH configuration files for a host using the built-in ssh2-config parser.
///
/// Reads both the system config (`/etc/ssh/ssh_config`) and the user config
/// (`~/.ssh/config`), merging them with user config taking precedence.
pub(crate) fn parse_ssh_config(host: &str) -> io::Result<ResolvedConfig> {
    use ssh2_config::DefaultAlgorithms;

    let system_params = crate::system_ssh_dir()
        .map(|d| d.join("ssh_config"))
        .and_then(|path| try_parse_ssh_config_file(&path, host));

    let user_params = dirs::home_dir()
        .map(|h| h.join(".ssh").join("config"))
        .and_then(|path| try_parse_ssh_config_file(&path, host));

    // Merge: user config takes precedence over system config
    let params = match (user_params, system_params) {
        (Some(mut user), Some(system)) => {
            user.overwrite_if_none(&system);
            user
        }
        (Some(user), None) => user,
        (None, Some(system)) => system,
        (None, None) => HostParams::new(&DefaultAlgorithms::default()),
    };

    Ok(ResolvedConfig::from_host_params(&params))
}

/// Try to parse an SSH config file and query it for a host.
/// Returns `None` if the file doesn't exist or can't be parsed.
pub(crate) fn try_parse_ssh_config_file(path: &Path, host: &str) -> Option<HostParams> {
    if !path.exists() {
        return None;
    }
    match File::open(path) {
        Ok(f) => {
            let mut reader = BufReader::new(f);
            match SshConfig::default().parse(&mut reader, ParseRule::ALLOW_UNSUPPORTED_FIELDS) {
                Ok(config) => Some(config.query(host)),
                Err(e) => {
                    debug!("Failed to parse SSH config {}: {}", path.display(), e);
                    None
                }
            }
        }
        Err(e) => {
            debug!("Failed to open SSH config {}: {}", path.display(), e);
            None
        }
    }
}

/// Build a russh client configuration from SSH options and resolved config.
pub(crate) fn build_russh_config(
    _opts: &SshOpts,
    config: &ResolvedConfig,
) -> io::Result<russh::client::Config> {
    let mut russh_config = russh::client::Config::default();

    russh_config.preferred = build_preferred_algorithms(config);

    // Map keepalive: prefer server_alive_interval, fall back to tcp_keep_alive
    if let Some(interval) = config.server_alive_interval {
        russh_config.keepalive_interval = Some(interval);
    } else if config.tcp_keep_alive == Some(true) {
        // TCP keepalive requested but no interval specified; use a sensible default
        russh_config.keepalive_interval = Some(Duration::from_secs(15));
    }

    // Map connection timeout
    if let Some(timeout) = config.connect_timeout {
        russh_config.inactivity_timeout = Some(timeout);
    }

    Ok(russh_config)
}

/// Builds preferred algorithm lists from resolved config, filtering to only
/// algorithms that russh actually supports. Unsupported algorithm names are
/// logged and skipped.
pub(crate) fn build_preferred_algorithms(config: &ResolvedConfig) -> russh::Preferred {
    use russh::keys::Algorithm;

    let mut preferred = russh::Preferred::default();

    // Map KexAlgorithms
    if !config.kex_algorithms.is_empty() {
        let kex: Vec<russh::kex::Name> = config
            .kex_algorithms
            .iter()
            .filter_map(|s| match russh::kex::Name::try_from(s.as_str()) {
                Ok(name) => Some(name),
                Err(_) => {
                    debug!("Skipping unsupported KEX algorithm from SSH config: {}", s);
                    None
                }
            })
            .collect();
        if !kex.is_empty() {
            // Append extension negotiation names that russh needs internally
            let mut full_kex = kex;
            for ext in [
                russh::kex::EXTENSION_SUPPORT_AS_CLIENT,
                russh::kex::EXTENSION_OPENSSH_STRICT_KEX_AS_CLIENT,
            ] {
                if !full_kex.contains(&ext) {
                    full_kex.push(ext);
                }
            }
            preferred.kex = full_kex.into();
        }
    }

    // Map HostKeyAlgorithms
    if !config.host_key_algorithms.is_empty() {
        let keys: Vec<Algorithm> = config
            .host_key_algorithms
            .iter()
            .filter_map(|s| match s.parse::<Algorithm>() {
                Ok(algo) if !matches!(algo, Algorithm::Other(_)) => Some(algo),
                Ok(_) => {
                    debug!(
                        "Skipping unsupported host key algorithm from SSH config: {}",
                        s
                    );
                    None
                }
                Err(_) => {
                    debug!(
                        "Skipping unrecognized host key algorithm from SSH config: {}",
                        s
                    );
                    None
                }
            })
            .collect();
        if !keys.is_empty() {
            preferred.key = keys.into();
        }
    }

    // Map Ciphers
    if !config.ciphers.is_empty() {
        let ciphers: Vec<russh::cipher::Name> = config
            .ciphers
            .iter()
            .filter_map(|s| match russh::cipher::Name::try_from(s.as_str()) {
                Ok(name) => Some(name),
                Err(_) => {
                    debug!("Skipping unsupported cipher from SSH config: {}", s);
                    None
                }
            })
            .collect();
        if !ciphers.is_empty() {
            preferred.cipher = ciphers.into();
        }
    }

    // Map MACs
    if !config.macs.is_empty() {
        let macs: Vec<russh::mac::Name> = config
            .macs
            .iter()
            .filter_map(|s| match russh::mac::Name::try_from(s.as_str()) {
                Ok(name) => Some(name),
                Err(_) => {
                    debug!("Skipping unsupported MAC from SSH config: {}", s);
                    None
                }
            })
            .collect();
        if !macs.is_empty() {
            preferred.mac = macs.into();
        }
    }

    // Map Compression
    if let Some(true) = config.compression {
        let compressed: Vec<russh::compression::Name> = ["zlib@openssh.com", "zlib", "none"]
            .iter()
            .filter_map(|s| russh::compression::Name::try_from(*s).ok())
            .collect();
        if !compressed.is_empty() {
            preferred.compression = compressed.into();
        }
    }

    preferred
}
