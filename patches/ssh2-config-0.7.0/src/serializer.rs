//! SSH Config serializer

use std::fmt;

use crate::{Host, HostClause, HostParams, SshConfig};

pub struct SshConfigSerializer<'a>(&'a SshConfig);

impl SshConfigSerializer<'_> {
    pub fn serialize(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.hosts.is_empty() {
            return Ok(());
        }

        // serialize first host
        let root = self.0.hosts.first().unwrap();
        // check if first host is the default host
        if root.pattern == vec![HostClause::new(String::from("*"), false)] {
            Self::serialize_host_params(f, &root.params, false)?;
        } else {
            Self::serialize_host(f, root)?;
        }

        // serialize other hosts
        for host in self.0.hosts.iter().skip(1) {
            Self::serialize_host(f, host)?;
        }

        Ok(())
    }

    fn serialize_host(f: &mut fmt::Formatter<'_>, host: &Host) -> fmt::Result {
        let patterns = &host
            .pattern
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        writeln!(f, "Host {patterns}",)?;
        Self::serialize_host_params(f, &host.params, true)?;
        writeln!(f,)?;

        Ok(())
    }

    fn serialize_host_params(
        f: &mut fmt::Formatter<'_>,
        params: &HostParams,
        nested: bool,
    ) -> fmt::Result {
        let padding = if nested { "    " } else { "" };

        if let Some(value) = params.bind_address.as_ref() {
            writeln!(f, "{padding}Hostname {value}",)?;
        }
        if let Some(add_keys_to_agent) = params.add_keys_to_agent.as_ref() {
            writeln!(
                f,
                "{padding}AddKeysToAgent {}",
                if *add_keys_to_agent { "yes" } else { "no" }
            )?;
        }
        if let Some(value) = params.bind_interface.as_ref() {
            writeln!(f, "{padding}BindAddress {value}",)?;
        }
        if !params.ca_signature_algorithms.is_default() {
            writeln!(
                f,
                "{padding}CASignatureAlgorithms {ca_signature_algorithms}",
                padding = padding,
                ca_signature_algorithms = params.ca_signature_algorithms
            )?;
        }
        if let Some(certificate_file) = params.certificate_file.as_ref() {
            writeln!(f, "{padding}CertificateFile {}", certificate_file.display())?;
        }
        if !params.ciphers.is_default() {
            writeln!(
                f,
                "{padding}Ciphers {ciphers}",
                padding = padding,
                ciphers = params.ciphers
            )?;
        }
        if let Some(value) = params.compression.as_ref() {
            writeln!(
                f,
                "{padding}Compression {}",
                if *value { "yes" } else { "no" }
            )?;
        }
        if let Some(connection_attempts) = params.connection_attempts {
            writeln!(f, "{padding}ConnectionAttempts {connection_attempts}",)?;
        }
        if let Some(connect_timeout) = params.connect_timeout {
            writeln!(f, "{padding}ConnectTimeout {}", connect_timeout.as_secs())?;
        }
        if let Some(forward_agent) = params.forward_agent.as_ref() {
            writeln!(
                f,
                "{padding}ForwardAgent {}",
                if *forward_agent { "yes" } else { "no" }
            )?;
        }
        if !params.host_key_algorithms.is_default() {
            writeln!(
                f,
                "{padding}HostKeyAlgorithms {host_key_algorithms}",
                padding = padding,
                host_key_algorithms = params.host_key_algorithms
            )?;
        }
        if let Some(host_name) = params.host_name.as_ref() {
            writeln!(f, "{padding}HostName {host_name}",)?;
        }
        if let Some(identity_file) = params.identity_file.as_ref() {
            writeln!(
                f,
                "{padding}IdentityFile {}",
                identity_file
                    .iter()
                    .map(|p| p.display().to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )?;
        }
        if let Some(ignore_unknown) = params.ignore_unknown.as_ref() {
            writeln!(
                f,
                "{padding}IgnoreUnknown {}",
                ignore_unknown
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )?;
        }
        if !params.kex_algorithms.is_default() {
            writeln!(
                f,
                "{padding}KexAlgorithms {kex_algorithms}",
                padding = padding,
                kex_algorithms = params.kex_algorithms
            )?;
        }
        if !params.mac.is_default() {
            writeln!(
                f,
                "{padding}MACs {mac}",
                padding = padding,
                mac = params.mac
            )?;
        }
        if let Some(port) = params.port {
            writeln!(f, "{padding}Port {port}", port = port)?;
        }
        if let Some(proxy_jump) = params.proxy_jump.as_ref() {
            writeln!(
                f,
                "{padding}ProxyJump {}",
                proxy_jump
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )?;
        }
        if !params.pubkey_accepted_algorithms.is_default() {
            writeln!(
                f,
                "{padding}PubkeyAcceptedAlgorithms {pubkey_accepted_algorithms}",
                padding = padding,
                pubkey_accepted_algorithms = params.pubkey_accepted_algorithms
            )?;
        }
        if let Some(pubkey_authentication) = params.pubkey_authentication.as_ref() {
            writeln!(
                f,
                "{padding}PubkeyAuthentication {}",
                if *pubkey_authentication { "yes" } else { "no" }
            )?;
        }
        if let Some(remote_forward) = params.remote_forward.as_ref() {
            writeln!(f, "{padding}RemoteForward {remote_forward}",)?;
        }
        if let Some(server_alive_interval) = params.server_alive_interval {
            writeln!(
                f,
                "{padding}ServerAliveInterval {}",
                server_alive_interval.as_secs()
            )?;
        }
        if let Some(tcp_keep_alive) = params.tcp_keep_alive.as_ref() {
            writeln!(
                f,
                "{padding}TCPKeepAlive {}",
                if *tcp_keep_alive { "yes" } else { "no" }
            )?;
        }
        #[cfg(target_os = "macos")]
        if let Some(use_keychain) = params.use_keychain.as_ref() {
            writeln!(
                f,
                "{padding}UseKeychain {}",
                if *use_keychain { "yes" } else { "no" }
            )?;
        }
        if let Some(user) = params.user.as_ref() {
            writeln!(f, "{padding}User {user}",)?;
        }
        for (field, value) in &params.ignored_fields {
            writeln!(
                f,
                "{padding}{field} {value}",
                field = field,
                value = value
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            )?;
        }
        for (field, value) in &params.unsupported_fields {
            writeln!(
                f,
                "{padding}{field} {value}",
                field = field,
                value = value
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            )?;
        }

        Ok(())
    }
}

impl<'a> From<&'a SshConfig> for SshConfigSerializer<'a> {
    fn from(config: &'a SshConfig) -> Self {
        SshConfigSerializer(config)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::{DefaultAlgorithms, HostClause};

    #[test]
    fn are_host_patterns_combined() {
        let mut host_params = HostParams::new(&DefaultAlgorithms::empty());
        host_params.host_name = Some("bastion.example.com".to_string());

        let host = Host::new(
            vec![
                HostClause::new(String::from("*.example.com"), false),
                HostClause::new(String::from("foo.example.com"), true),
            ],
            host_params,
        );

        let output = SshConfig::from_hosts(vec![/*default_host,*/ host]).to_string();
        assert!(&output.contains("Host *.example.com !foo.example.com"));
    }

    #[test]
    fn is_default_host_serialized_without_host() {
        let mut root_params = HostParams::new(&DefaultAlgorithms::empty());
        root_params.server_alive_interval = Some(Duration::from_secs(60));
        let root = Host::new(vec![HostClause::new(String::from("*"), false)], root_params);

        let mut host_params = HostParams::new(&DefaultAlgorithms::empty());
        host_params.user = Some("example".to_string());
        let host = Host::new(
            vec![HostClause::new(String::from("*.example.com"), false)],
            host_params,
        );

        let output = SshConfig::from_hosts(vec![root, host]).to_string();
        assert!(&output.starts_with("ServerAliveInterval 60"));
    }

    #[test]
    fn serialize_empty_config() {
        let config = SshConfig::from_hosts(vec![]);
        let output = config.to_string();
        assert!(output.is_empty());
    }

    #[test]
    fn serialize_first_host_not_default() {
        // When first host is not a default "*" pattern, it should be serialized with Host directive
        let mut host_params = HostParams::new(&DefaultAlgorithms::empty());
        host_params.user = Some("user".to_string());
        let host = Host::new(
            vec![HostClause::new(String::from("example.com"), false)],
            host_params,
        );

        let output = SshConfig::from_hosts(vec![host]).to_string();
        assert!(output.starts_with("Host example.com"));
        assert!(output.contains("User user"));
    }

    #[test]
    fn serialize_all_supported_fields() {
        use std::path::PathBuf;

        let mut params = HostParams::new(&DefaultAlgorithms::empty());
        params.bind_address = Some("10.0.0.1".to_string());
        params.add_keys_to_agent = Some(true);
        params.bind_interface = Some("eth0".to_string());
        params.certificate_file = Some(PathBuf::from("/path/to/cert"));
        params.compression = Some(true);
        params.connection_attempts = Some(3);
        params.connect_timeout = Some(Duration::from_secs(30));
        params.forward_agent = Some(true);
        params.host_name = Some("real-host.com".to_string());
        params.identity_file = Some(vec![PathBuf::from("/path/to/id")]);
        params.ignore_unknown = Some(vec!["Field1".to_string(), "Field2".to_string()]);
        params.port = Some(22);
        params.proxy_jump = Some(vec!["jump1".to_string(), "jump2".to_string()]);
        params.pubkey_authentication = Some(false);
        params.remote_forward = Some(8080);
        params.server_alive_interval = Some(Duration::from_secs(60));
        params.tcp_keep_alive = Some(false);
        #[cfg(target_os = "macos")]
        {
            params.use_keychain = Some(true);
        }
        params.user = Some("testuser".to_string());
        params
            .ignored_fields
            .insert("CustomField".to_string(), vec!["value".to_string()]);
        params
            .unsupported_fields
            .insert("UnsupportedField".to_string(), vec!["val".to_string()]);

        let host = Host::new(
            vec![HostClause::new(String::from("test-host"), false)],
            params,
        );

        let output = SshConfig::from_hosts(vec![host]).to_string();

        assert!(output.contains("Host test-host"));
        assert!(output.contains("Hostname 10.0.0.1"));
        assert!(output.contains("AddKeysToAgent yes"));
        assert!(output.contains("BindAddress eth0"));
        assert!(output.contains("CertificateFile /path/to/cert"));
        assert!(output.contains("Compression yes"));
        assert!(output.contains("ConnectionAttempts 3"));
        assert!(output.contains("ConnectTimeout 30"));
        assert!(output.contains("ForwardAgent yes"));
        assert!(output.contains("HostName real-host.com"));
        assert!(output.contains("IdentityFile /path/to/id"));
        assert!(output.contains("IgnoreUnknown Field1,Field2"));
        assert!(output.contains("Port 22"));
        assert!(output.contains("ProxyJump jump1,jump2"));
        assert!(output.contains("PubkeyAuthentication no"));
        assert!(output.contains("RemoteForward 8080"));
        assert!(output.contains("ServerAliveInterval 60"));
        assert!(output.contains("TCPKeepAlive no"));
        #[cfg(target_os = "macos")]
        assert!(output.contains("UseKeychain yes"));
        assert!(output.contains("User testuser"));
        assert!(output.contains("CustomField value"));
        assert!(output.contains("UnsupportedField val"));
    }

    #[test]
    fn serialize_algorithms_with_rules() {
        use std::io::BufReader;

        use crate::{ParseRule, SshConfig};

        // Parse a config with algorithm rules to test serialization
        let config_str = r#"
Host algo-host
    Ciphers +aes256-ctr
    HostKeyAlgorithms ^ssh-rsa
    KexAlgorithms -diffie
    MACs hmac-sha2-256
    PubkeyAcceptedAlgorithms +ssh-ed25519
    CASignatureAlgorithms ecdsa-sha2-nistp256
"#;
        let mut reader = BufReader::new(config_str.as_bytes());
        let config = SshConfig::default()
            .default_algorithms(DefaultAlgorithms::empty())
            .parse(&mut reader, ParseRule::STRICT)
            .unwrap();

        let output = config.to_string();

        assert!(output.contains("Ciphers +"));
        assert!(output.contains("HostKeyAlgorithms ^"));
        assert!(output.contains("KexAlgorithms -"));
        assert!(output.contains("MACs"));
        assert!(output.contains("PubkeyAcceptedAlgorithms +"));
        assert!(output.contains("CASignatureAlgorithms"));
    }

    #[test]
    fn serialize_boolean_fields_as_no() {
        let mut params = HostParams::new(&DefaultAlgorithms::empty());
        params.add_keys_to_agent = Some(false);
        params.compression = Some(false);
        params.forward_agent = Some(false);
        params.tcp_keep_alive = Some(false);
        params.pubkey_authentication = Some(false);
        #[cfg(target_os = "macos")]
        {
            params.use_keychain = Some(false);
        }

        let host = Host::new(
            vec![HostClause::new(String::from("bool-host"), false)],
            params,
        );

        let output = SshConfig::from_hosts(vec![host]).to_string();

        assert!(output.contains("AddKeysToAgent no"));
        assert!(output.contains("Compression no"));
        assert!(output.contains("ForwardAgent no"));
        assert!(output.contains("TCPKeepAlive no"));
        assert!(output.contains("PubkeyAuthentication no"));
        #[cfg(target_os = "macos")]
        assert!(output.contains("UseKeychain no"));
    }

    #[test]
    fn serialize_multiple_hosts() {
        let mut params1 = HostParams::new(&DefaultAlgorithms::empty());
        params1.user = Some("user1".to_string());
        let host1 = Host::new(vec![HostClause::new(String::from("host1"), false)], params1);

        let mut params2 = HostParams::new(&DefaultAlgorithms::empty());
        params2.user = Some("user2".to_string());
        let host2 = Host::new(vec![HostClause::new(String::from("host2"), false)], params2);

        let output = SshConfig::from_hosts(vec![host1, host2]).to_string();

        assert!(output.contains("Host host1"));
        assert!(output.contains("User user1"));
        assert!(output.contains("Host host2"));
        assert!(output.contains("User user2"));
    }

    #[test]
    fn serialize_with_empty_params() {
        let params = HostParams::new(&DefaultAlgorithms::empty());
        let host = Host::new(
            vec![HostClause::new(String::from("empty-host"), false)],
            params,
        );

        let output = SshConfig::from_hosts(vec![host]).to_string();
        // Should only contain Host directive and newline
        assert!(output.starts_with("Host empty-host"));
    }
}
