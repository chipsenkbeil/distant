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

    /// Resolve SSH config for the given host.
    ///
    /// Tries `ssh -G` first (handles Match blocks, Include directives, etc.),
    /// then falls back to the built-in ssh2-config parser.
    pub(crate) async fn for_host(host: &str) -> io::Result<Self> {
        match query_openssh(host).await {
            Some(cfg) => {
                log::debug!("SSH config resolved via ssh -G");
                Ok(cfg)
            }
            None => {
                log::debug!("ssh -G unavailable, falling back to ssh2-config parser");
                parse_ssh_config(host)
            }
        }
    }

    /// Convert to a russh client configuration.
    pub(crate) fn to_russh_config(&self) -> io::Result<russh::client::Config> {
        let mut russh_config = russh::client::Config::default();

        russh_config.preferred = self.preferred_algorithms();

        // Map keepalive: prefer server_alive_interval, fall back to tcp_keep_alive
        if let Some(interval) = self.server_alive_interval {
            russh_config.keepalive_interval = Some(interval);
        } else if self.tcp_keep_alive == Some(true) {
            // TCP keepalive requested but no interval specified; use a sensible default
            russh_config.keepalive_interval = Some(Duration::from_secs(15));
        }

        // Map connection timeout
        if let Some(timeout) = self.connect_timeout {
            russh_config.inactivity_timeout = Some(timeout);
        }

        Ok(russh_config)
    }

    /// Builds preferred algorithm lists from resolved config, filtering to only
    /// algorithms that russh actually supports. Unsupported algorithm names are
    /// logged and skipped.
    fn preferred_algorithms(&self) -> russh::Preferred {
        use russh::keys::Algorithm;

        let mut preferred = russh::Preferred::default();

        // Map KexAlgorithms
        if !self.kex_algorithms.is_empty() {
            let kex: Vec<russh::kex::Name> = self
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
        if !self.host_key_algorithms.is_empty() {
            let keys: Vec<Algorithm> = self
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
        if !self.ciphers.is_empty() {
            let ciphers: Vec<russh::cipher::Name> = self
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
        if !self.macs.is_empty() {
            let macs: Vec<russh::mac::Name> = self
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
        if let Some(true) = self.compression {
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
}

/// Query OpenSSH for resolved config via `ssh -G hostname`.
///
/// This delegates to the system's `ssh` binary, which handles `Match` blocks,
/// `Include` directives, and other advanced config features that the built-in
/// ssh2-config parser does not support.
///
/// Returns `None` if `ssh` is not available or the command fails.
async fn query_openssh(host: &str) -> Option<ResolvedConfig> {
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
fn parse_ssh_g_output(output: &str) -> Option<ResolvedConfig> {
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
fn parse_ssh_config(host: &str) -> io::Result<ResolvedConfig> {
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
fn try_parse_ssh_config_file(path: &Path, host: &str) -> Option<HostParams> {
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

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::Path;
    use std::time::Duration;

    use rstest::rstest;
    use ssh2_config::{ParseRule, SshConfig};

    use super::*;

    /// Helper: parse a temp SSH config and return a ResolvedConfig for testing.
    fn parse_config_str(config_text: &str) -> ResolvedConfig {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "{}", config_text).unwrap();

        let mut reader = std::io::BufReader::new(std::fs::File::open(&path).unwrap());
        let ssh_config = SshConfig::default()
            .parse(&mut reader, ParseRule::ALLOW_UNSUPPORTED_FIELDS)
            .unwrap();
        ResolvedConfig::from_host_params(&ssh_config.query("testhost"))
    }

    #[test]
    fn preferred_algorithms_should_return_defaults_with_empty_params() {
        let resolved = ResolvedConfig::default();
        let preferred = resolved.preferred_algorithms();

        let default_preferred = russh::Preferred::default();
        assert_eq!(preferred.kex, default_preferred.kex);
        assert_eq!(preferred.cipher, default_preferred.cipher);
    }

    #[test]
    fn preferred_algorithms_should_use_defaults_despite_custom_params() {
        let mut resolved = ResolvedConfig::default();
        resolved.port = Some(9999);
        resolved.user = Some("custom-user".to_string());

        let preferred = resolved.preferred_algorithms();
        let default_preferred = russh::Preferred::default();
        assert_eq!(preferred.kex, default_preferred.kex);
    }

    #[test]
    fn preferred_algorithms_should_map_custom_ciphers() {
        let params = parse_config_str(
            "Host testhost\n  Ciphers chacha20-poly1305@openssh.com,aes256-gcm@openssh.com\n",
        );

        let preferred = params.preferred_algorithms();
        assert!(preferred.cipher.len() <= 2);
        assert!(
            preferred
                .cipher
                .iter()
                .any(|c| c.as_ref() == "chacha20-poly1305@openssh.com")
        );
    }

    #[test]
    fn preferred_algorithms_should_skip_unsupported_cipher() {
        let params = parse_config_str(
            "Host testhost\n  Ciphers aes256-gcm@openssh.com,nonexistent-cipher\n",
        );

        let preferred = params.preferred_algorithms();
        assert!(
            preferred
                .cipher
                .iter()
                .all(|c| c.as_ref() != "nonexistent-cipher")
        );
    }

    #[test]
    fn preferred_algorithms_should_map_custom_kex() {
        let params = parse_config_str("Host testhost\n  KexAlgorithms curve25519-sha256\n");

        let preferred = params.preferred_algorithms();
        assert!(
            preferred
                .kex
                .iter()
                .any(|k| k.as_ref() == "curve25519-sha256")
        );
    }

    #[test]
    fn preferred_algorithms_should_map_custom_mac() {
        let params = parse_config_str("Host testhost\n  MACs hmac-sha2-256\n");

        let preferred = params.preferred_algorithms();
        assert!(preferred.mac.iter().any(|m| m.as_ref() == "hmac-sha2-256"));
    }

    #[test]
    fn preferred_algorithms_should_filter_cert_host_key_algorithms() {
        let params = parse_config_str(
            "Host testhost\n  HostKeyAlgorithms ssh-ed25519-cert-v01@openssh.com,rsa-sha2-512-cert-v01@openssh.com,ssh-ed25519,rsa-sha2-512\n",
        );

        let preferred = params.preferred_algorithms();

        // Only the plain (non-cert) algorithms should survive filtering.
        // Cert variants parse as Algorithm::Other(...) and must be rejected.
        assert_eq!(
            preferred.key.as_ref(),
            &[
                russh::keys::Algorithm::Ed25519,
                russh::keys::Algorithm::Rsa {
                    hash: Some(russh::keys::HashAlg::Sha512),
                },
            ]
        );
    }

    #[test]
    fn preferred_algorithms_should_keep_plain_host_key_algorithms() {
        let params =
            parse_config_str("Host testhost\n  HostKeyAlgorithms ssh-ed25519,rsa-sha2-256\n");

        let preferred = params.preferred_algorithms();

        // All plain algorithms are recognized by russh and should be preserved.
        assert_eq!(
            preferred.key.as_ref(),
            &[
                russh::keys::Algorithm::Ed25519,
                russh::keys::Algorithm::Rsa {
                    hash: Some(russh::keys::HashAlg::Sha256),
                },
            ]
        );
    }

    #[rstest]
    #[case::nonexistent("nonexistent-host.example.com")]
    #[case::localhost("localhost")]
    #[case::wildcard("*")]
    #[case::empty("")]
    #[case::ipv4("192.168.1.1")]
    #[case::ipv6("::1")]
    #[case::fqdn("server.example.co.uk")]
    #[case::hyphenated("my-server-01.internal")]
    #[case::underscore("my_server_01")]
    fn parse_ssh_config_should_not_error_for_any_hostname(#[case] host: &str) {
        // parse_ssh_config reads the real ~/.ssh/config (or returns defaults).
        // We can't assert specific field values since the user's config may have
        // wildcard matches. The unwrap proves it never errors for valid input.
        let _params = parse_ssh_config(host).unwrap();
    }

    #[test]
    fn try_parse_ssh_config_file_should_return_none_for_missing_file() {
        let result = try_parse_ssh_config_file(Path::new("/nonexistent/path/config"), "host");
        assert!(result.is_none());
    }

    #[test]
    fn try_parse_ssh_config_file_should_parse_valid_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("ssh_config");
        let mut f = std::fs::File::create(&config_path).unwrap();
        writeln!(f, "Host testhost\n  HostName 10.0.0.1\n  Port 2222").unwrap();

        let result = try_parse_ssh_config_file(&config_path, "testhost");
        assert!(result.is_some());
        let params = result.unwrap();
        assert_eq!(params.host_name.as_deref(), Some("10.0.0.1"));
        assert_eq!(params.port, Some(2222));
    }

    #[test]
    fn try_parse_ssh_config_file_should_not_panic_on_invalid_content() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("bad_config");
        // Write binary garbage that can't be parsed as SSH config
        std::fs::write(&config_path, [0xFF, 0xFE, 0x00, 0x01]).unwrap();

        let result = try_parse_ssh_config_file(&config_path, "host");

        // ssh2-config is fairly permissive — it may return Some with empty params
        // or None on parse failure. Either is acceptable; the key assertion is that
        // the function does not panic on binary input.
        if let Some(params) = result {
            // If parsed, the garbage should not produce meaningful SSH settings
            assert!(
                params.host_name.is_none(),
                "Binary garbage should not produce a valid HostName"
            );
        }
    }

    #[test]
    fn try_parse_ssh_config_file_should_extract_unsupported_fields() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("ssh_config");
        let mut f = std::fs::File::create(&config_path).unwrap();
        writeln!(
            f,
            "Host corp-vm\n  ProxyCommand exec ssh -W %h:%p jump\n  IdentitiesOnly yes"
        )
        .unwrap();

        let params = try_parse_ssh_config_file(&config_path, "corp-vm").unwrap();

        // ssh2-config splits unsupported field values into words
        let proxy = params
            .unsupported_fields
            .get("proxycommand")
            .map(|v| v.join(" "));
        assert_eq!(
            proxy.as_deref(),
            Some("exec ssh -W %h:%p jump"),
            "ProxyCommand should be in unsupported_fields (rejoined)"
        );

        let id_only = params
            .unsupported_fields
            .get("identitiesonly")
            .and_then(|v| v.first());
        assert_eq!(
            id_only.map(|s| s.as_str()),
            Some("yes"),
            "IdentitiesOnly should be in unsupported_fields"
        );
    }

    #[test]
    fn parse_ssh_config_should_merge_system_and_user_configs() {
        // This test verifies the merge behavior by testing try_parse_ssh_config_file
        // directly, since parse_ssh_config reads from fixed system paths
        let dir = tempfile::tempdir().unwrap();

        // Simulate system config (has HostName but not User)
        let system_path = dir.path().join("system_config");
        let mut f = std::fs::File::create(&system_path).unwrap();
        writeln!(f, "Host myhost\n  HostName 10.0.0.1\n  Port 22").unwrap();

        // Simulate user config (has User but not HostName)
        let user_path = dir.path().join("user_config");
        let mut f = std::fs::File::create(&user_path).unwrap();
        writeln!(f, "Host myhost\n  User deployer\n  Port 2222").unwrap();

        let system_params = try_parse_ssh_config_file(&system_path, "myhost").unwrap();
        let mut user_params = try_parse_ssh_config_file(&user_path, "myhost").unwrap();

        // User has Port=2222 but no HostName; system has HostName and Port=22
        assert_eq!(user_params.port, Some(2222));
        assert!(user_params.host_name.is_none());
        assert_eq!(system_params.host_name.as_deref(), Some("10.0.0.1"));

        // After merge, user values take precedence
        user_params.overwrite_if_none(&system_params);
        assert_eq!(user_params.port, Some(2222), "User port should win");
        assert_eq!(
            user_params.host_name.as_deref(),
            Some("10.0.0.1"),
            "System HostName should fill in missing user HostName"
        );
        assert_eq!(
            user_params.user.as_deref(),
            Some("deployer"),
            "User should be preserved"
        );
    }

    #[test]
    fn parse_ssh_g_output_should_extract_basic_fields() {
        let output = "\
hostname devvm24531.example.com
user example
port 22
identityfile ~/.ssh/id_rsa
identityfile ~/.ssh/id_ed25519
proxycommand x2ssh -fallback -tunnel %h
identitiesonly no
";
        let params = parse_ssh_g_output(output).unwrap();

        assert_eq!(params.host_name.as_deref(), Some("devvm24531.example.com"));
        assert_eq!(params.user.as_deref(), Some("example"));
        assert_eq!(params.port, Some(22));

        assert_eq!(params.identity_files.len(), 2);
        assert_eq!(params.identity_files[0], PathBuf::from("~/.ssh/id_rsa"));
        assert_eq!(params.identity_files[1], PathBuf::from("~/.ssh/id_ed25519"));

        assert_eq!(
            params.proxy_command.as_deref(),
            Some("x2ssh -fallback -tunnel %h")
        );

        assert_eq!(params.identities_only.as_deref(), Some("no"));
    }

    #[test]
    fn parse_ssh_g_output_should_skip_proxycommand_none() {
        let output = "\
hostname example.com
proxycommand none
port 22
";
        let params = parse_ssh_g_output(output).unwrap();

        assert!(
            params.proxy_command.is_none(),
            "proxycommand 'none' should not be stored"
        );
        assert_eq!(params.host_name.as_deref(), Some("example.com"));
        assert_eq!(params.port, Some(22));
    }

    #[test]
    fn parse_ssh_g_output_should_skip_proxycommand_none_case_insensitive() {
        let output = "proxycommand NONE\nhostname host.example.com\n";
        let params = parse_ssh_g_output(output).unwrap();

        assert!(
            params.proxy_command.is_none(),
            "proxycommand 'NONE' (uppercase) should not be stored"
        );
    }

    #[test]
    fn parse_ssh_g_output_should_parse_algorithms() {
        let output = "\
hostname algo-host.example.com
ciphers chacha20-poly1305@openssh.com,aes256-gcm@openssh.com,aes128-gcm@openssh.com
kexalgorithms sntrup761x25519-sha512@openssh.com,curve25519-sha256
hostkeyalgorithms ssh-ed25519-cert-v01@openssh.com,ssh-ed25519
macs umac-128-etm@openssh.com,hmac-sha2-256-etm@openssh.com
pubkeyacceptedalgorithms ssh-ed25519-cert-v01@openssh.com,ssh-ed25519
";
        let params = parse_ssh_g_output(output).unwrap();

        assert!(
            !params.ciphers.is_empty(),
            "ciphers should be overridden (not empty)"
        );
        assert_eq!(
            &params.ciphers,
            &[
                "chacha20-poly1305@openssh.com",
                "aes256-gcm@openssh.com",
                "aes128-gcm@openssh.com",
            ]
        );

        assert!(
            !params.kex_algorithms.is_empty(),
            "kexalgorithms should be overridden"
        );
        assert_eq!(
            &params.kex_algorithms,
            &["sntrup761x25519-sha512@openssh.com", "curve25519-sha256",]
        );

        assert!(
            !params.host_key_algorithms.is_empty(),
            "hostkeyalgorithms should be overridden"
        );
        assert_eq!(
            &params.host_key_algorithms,
            &["ssh-ed25519-cert-v01@openssh.com", "ssh-ed25519",]
        );

        assert!(!params.macs.is_empty(), "macs should be overridden");
        assert_eq!(
            &params.macs,
            &["umac-128-etm@openssh.com", "hmac-sha2-256-etm@openssh.com",]
        );

        assert!(
            !params.pubkey_accepted_algorithms.is_empty(),
            "pubkeyacceptedalgorithms should be overridden"
        );
        assert_eq!(
            &params.pubkey_accepted_algorithms,
            &["ssh-ed25519-cert-v01@openssh.com", "ssh-ed25519",]
        );
    }

    #[test]
    fn parse_ssh_g_output_should_return_none_for_empty_output() {
        assert!(
            parse_ssh_g_output("").is_none(),
            "Empty string should return None"
        );
    }

    #[test]
    fn parse_ssh_g_output_should_return_none_for_whitespace_only() {
        assert!(
            parse_ssh_g_output("   \n  \n\n").is_none(),
            "Whitespace-only input should return None"
        );
    }

    #[test]
    fn parse_ssh_g_output_should_parse_proxy_jump() {
        let output = "\
hostname target.example.com
proxyjump jump1.example.com,jump2.example.com:2222,user@jump3.example.com
port 22
";
        let params = parse_ssh_g_output(output).unwrap();

        let proxy_jump = params.proxy_jump.as_ref().unwrap();
        assert_eq!(proxy_jump.len(), 3);
        assert_eq!(proxy_jump[0], "jump1.example.com");
        assert_eq!(proxy_jump[1], "jump2.example.com:2222");
        assert_eq!(proxy_jump[2], "user@jump3.example.com");
    }

    #[test]
    fn parse_ssh_g_output_should_skip_proxyjump_none() {
        let output = "hostname host.example.com\nproxyjump none\n";
        let params = parse_ssh_g_output(output).unwrap();
        assert!(
            params.proxy_jump.is_none(),
            "proxyjump 'none' should not populate proxy_jump"
        );
    }

    #[test]
    fn parse_ssh_g_output_should_parse_known_hosts_files() {
        let output = "\
hostname kh-host.example.com
userknownhostsfile /Users/user/.ssh/known_hosts /Users/user/.ssh/known_hosts2
globalknownhostsfile /etc/ssh/ssh_known_hosts /etc/ssh/ssh_known_hosts2
";
        let params = parse_ssh_g_output(output).unwrap();

        assert_eq!(
            params.user_known_hosts_files,
            &[
                "/Users/user/.ssh/known_hosts",
                "/Users/user/.ssh/known_hosts2",
            ]
        );

        assert_eq!(
            params.global_known_hosts_files,
            &["/etc/ssh/ssh_known_hosts", "/etc/ssh/ssh_known_hosts2",]
        );
    }

    #[test]
    fn parse_ssh_g_output_should_parse_keepalive_and_timeout() {
        let output = "\
hostname keepalive-host.example.com
serveraliveinterval 60
tcpkeepalive yes
connecttimeout 30
compression no
";
        let params = parse_ssh_g_output(output).unwrap();

        assert_eq!(params.server_alive_interval, Some(Duration::from_secs(60)));
        assert_eq!(params.tcp_keep_alive, Some(true));
        assert_eq!(params.connect_timeout, Some(Duration::from_secs(30)));
        assert_eq!(params.compression, Some(false));
    }

    #[test]
    fn parse_ssh_g_output_should_skip_zero_intervals() {
        let output = "\
hostname zero-host.example.com
serveraliveinterval 0
connecttimeout 0
tcpkeepalive no
compression yes
";
        let params = parse_ssh_g_output(output).unwrap();

        assert!(
            params.server_alive_interval.is_none(),
            "serveraliveinterval 0 should remain None"
        );
        assert!(
            params.connect_timeout.is_none(),
            "connecttimeout 0 should remain None"
        );
        assert_eq!(params.tcp_keep_alive, Some(false));
        assert_eq!(params.compression, Some(true));
    }

    #[test]
    fn parse_ssh_g_output_should_parse_stricthostkeychecking() {
        let output = "hostname strict-host.example.com\nstricthostkeychecking accept-new\n";
        let params = parse_ssh_g_output(output).unwrap();

        assert_eq!(
            params.strict_host_key_checking.as_deref(),
            Some("accept-new")
        );
    }

    #[test]
    fn parse_ssh_g_output_should_ignore_unrecognized_keys() {
        let output = "\
hostname recognized.example.com
somefuturekey some-value
anothernewkey another-value
port 2222
";
        let params = parse_ssh_g_output(output).unwrap();

        assert_eq!(params.host_name.as_deref(), Some("recognized.example.com"));
        assert_eq!(params.port, Some(2222));

        // Unrecognized keys are silently ignored; ResolvedConfig has no catch-all field
    }

    #[test]
    fn parse_ssh_g_output_should_skip_lines_without_space() {
        let output = "hostname correct.example.com\nmalformedline\nport 443\n";
        let params = parse_ssh_g_output(output).unwrap();

        assert_eq!(params.host_name.as_deref(), Some("correct.example.com"));
        assert_eq!(params.port, Some(443));
    }

    #[test]
    fn parse_ssh_g_output_should_handle_invalid_port() {
        let output = "hostname bad-port.example.com\nport notanumber\n";
        let params = parse_ssh_g_output(output).unwrap();

        assert_eq!(params.host_name.as_deref(), Some("bad-port.example.com"));
        assert!(
            params.port.is_none(),
            "Non-numeric port should leave port as None"
        );
    }

    #[test]
    fn parse_ssh_g_output_should_parse_single_proxy_jump_hop() {
        let output = "hostname target.example.com\nproxyjump bastion.example.com\n";
        let params = parse_ssh_g_output(output).unwrap();

        let proxy_jump = params.proxy_jump.as_ref().unwrap();
        assert_eq!(proxy_jump.len(), 1);
        assert_eq!(proxy_jump[0], "bastion.example.com");
    }

    #[test]
    fn parse_ssh_g_output_should_parse_full_representative_output() {
        let output = "\
hostname devvm24531.example.com
user example
port 22
identityfile ~/.ssh/id_rsa
identityfile ~/.ssh/id_ed25519
proxycommand x2ssh -fallback -tunnel %h
identitiesonly no
userknownhostsfile /Users/user/.ssh/known_hosts /Users/user/.ssh/known_hosts2
globalknownhostsfile /etc/ssh/ssh_known_hosts /etc/ssh/ssh_known_hosts2
stricthostkeychecking accept-new
serveraliveinterval 0
tcpkeepalive yes
connecttimeout 30
compression no
ciphers chacha20-poly1305@openssh.com,aes256-gcm@openssh.com,aes128-gcm@openssh.com
kexalgorithms sntrup761x25519-sha512@openssh.com,curve25519-sha256
hostkeyalgorithms ssh-ed25519-cert-v01@openssh.com,ssh-ed25519
macs umac-128-etm@openssh.com,hmac-sha2-256-etm@openssh.com
pubkeyacceptedalgorithms ssh-ed25519-cert-v01@openssh.com,ssh-ed25519
";
        let params = parse_ssh_g_output(output).unwrap();

        assert_eq!(params.host_name.as_deref(), Some("devvm24531.example.com"));
        assert_eq!(params.user.as_deref(), Some("example"));
        assert_eq!(params.port, Some(22));
        assert_eq!(params.identity_files.len(), 2);
        assert_eq!(
            params.proxy_command.as_deref(),
            Some("x2ssh -fallback -tunnel %h")
        );
        assert_eq!(params.identities_only.as_deref(), Some("no"));
        assert_eq!(
            params.user_known_hosts_files,
            &[
                "/Users/user/.ssh/known_hosts",
                "/Users/user/.ssh/known_hosts2",
            ]
        );
        assert_eq!(
            params.global_known_hosts_files,
            &["/etc/ssh/ssh_known_hosts", "/etc/ssh/ssh_known_hosts2"]
        );
        assert_eq!(
            params.strict_host_key_checking.as_deref(),
            Some("accept-new")
        );
        assert!(params.server_alive_interval.is_none());
        assert_eq!(params.tcp_keep_alive, Some(true));
        assert_eq!(params.connect_timeout, Some(Duration::from_secs(30)));
        assert_eq!(params.compression, Some(false));
        assert_eq!(
            params.ciphers,
            &[
                "chacha20-poly1305@openssh.com",
                "aes256-gcm@openssh.com",
                "aes128-gcm@openssh.com",
            ]
        );
        assert_eq!(
            params.kex_algorithms,
            &["sntrup761x25519-sha512@openssh.com", "curve25519-sha256"]
        );
        assert_eq!(
            params.host_key_algorithms,
            &["ssh-ed25519-cert-v01@openssh.com", "ssh-ed25519"]
        );
        assert_eq!(
            params.macs,
            &["umac-128-etm@openssh.com", "hmac-sha2-256-etm@openssh.com"]
        );
        assert_eq!(
            params.pubkey_accepted_algorithms,
            &["ssh-ed25519-cert-v01@openssh.com", "ssh-ed25519"]
        );
    }

    #[test]
    fn parse_ssh_g_output_should_keep_default_algorithms_when_not_present() {
        let output = "hostname minimal.example.com\nport 22\n";
        let params = parse_ssh_g_output(output).unwrap();

        assert!(
            params.ciphers.is_empty(),
            "ciphers should remain empty when not in output"
        );
        assert!(
            params.kex_algorithms.is_empty(),
            "kex_algorithms should remain empty when not in output"
        );
        assert!(
            params.host_key_algorithms.is_empty(),
            "host_key_algorithms should remain empty when not in output"
        );
        assert!(
            params.macs.is_empty(),
            "macs should remain empty when not in output"
        );
        assert!(
            params.pubkey_accepted_algorithms.is_empty(),
            "pubkey_accepted_algorithms should remain empty when not in output"
        );
    }

    #[test]
    fn parse_ssh_g_output_should_parse_identity_agent() {
        let output = "\
hostname agent-host.example.com
identityagent /tmp/ssh-agent.sock
port 22
";
        let params = parse_ssh_g_output(output).unwrap();

        assert_eq!(
            params.identity_agent.as_deref(),
            Some("/tmp/ssh-agent.sock")
        );
    }

    #[test]
    fn parse_ssh_g_output_should_skip_identity_agent_none() {
        let output = "hostname agent-host.example.com\nidentityagent none\n";
        let params = parse_ssh_g_output(output).unwrap();

        assert!(
            params.identity_agent.is_none(),
            "identityagent 'none' should not be stored"
        );
    }

    #[test]
    fn parse_ssh_g_output_should_skip_identity_agent_none_case_insensitive() {
        for value in ["NONE", "None", "NoNe"] {
            let output = format!("hostname agent-host.example.com\nidentityagent {value}\n");
            let params = parse_ssh_g_output(&output).unwrap();

            assert!(
                params.identity_agent.is_none(),
                "identityagent '{value}' should not be stored"
            );
        }
    }

    #[test]
    fn to_russh_config_should_set_inactivity_timeout_from_connect_timeout() {
        let mut resolved = ResolvedConfig::default();
        resolved.connect_timeout = Some(Duration::from_secs(30));

        let cfg = resolved.to_russh_config().unwrap();
        assert_eq!(cfg.inactivity_timeout, Some(Duration::from_secs(30)));
    }

    #[test]
    fn preferred_algorithms_should_enable_compression_when_configured() {
        let mut resolved = ResolvedConfig::default();
        resolved.compression = Some(true);

        let preferred = resolved.preferred_algorithms();
        let default_preferred = russh::Preferred::default();
        assert_ne!(
            preferred.compression, default_preferred.compression,
            "compression should differ from defaults when enabled"
        );
        assert!(
            preferred
                .compression
                .iter()
                .any(|c| c.as_ref().contains("zlib")),
            "compression should include a zlib variant"
        );
    }
}
