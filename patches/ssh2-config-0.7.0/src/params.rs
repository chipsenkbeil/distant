//! # params
//!
//! Ssh config params for host rule

mod algos;

use std::collections::HashMap;

pub use self::algos::Algorithms;
pub(crate) use self::algos::AlgorithmsRule;
use super::{Duration, PathBuf};
use crate::DefaultAlgorithms;

/// Describes the ssh configuration.
/// Configuration is described in this document: <http://man.openbsd.org/OpenBSD-current/man5/ssh_config.5>
/// Only arguments supported by libssh2 are implemented
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostParams {
    /// Specifies whether keys should be automatically added to a running ssh-agent(1)
    pub add_keys_to_agent: Option<bool>,
    /// Specifies to use the specified address on the local machine as the source address of the connection
    pub bind_address: Option<String>,
    /// Use the specified address on the local machine as the source address of the connection
    pub bind_interface: Option<String>,
    /// Specifies which algorithms are allowed for signing of certificates by certificate authorities
    pub ca_signature_algorithms: Algorithms,
    /// Specifies a file from which the user's certificate is read
    pub certificate_file: Option<PathBuf>,
    /// Specifies the ciphers allowed for protocol version 2 in order of preference
    pub ciphers: Algorithms,
    /// Specifies whether to use compression
    pub compression: Option<bool>,
    /// Specifies the number of attempts to make before exiting
    pub connection_attempts: Option<usize>,
    /// Specifies the timeout used when connecting to the SSH server
    pub connect_timeout: Option<Duration>,
    /// Specifies whether the connection to the authentication agent (if any) will be forwarded to the remote machine
    pub forward_agent: Option<bool>,
    /// Specifies the host key signature algorithms that the client wants to use in order of preference
    pub host_key_algorithms: Algorithms,
    /// Specifies the real host name to log into
    pub host_name: Option<String>,
    /// Specifies the path of the identity file to be used when authenticating.
    /// More than one file can be specified.
    /// If more than one file is specified, they will be read in order.
    /// There may be multiple lines of this directive; in case subsequent lines will be appended to the previous ones.
    pub identity_file: Option<Vec<PathBuf>>,
    /// Specifies a pattern-list of unknown options to be ignored if they are encountered in configuration parsing
    pub ignore_unknown: Option<Vec<String>>,
    /// Specifies the available KEX (Key Exchange) algorithms
    pub kex_algorithms: Algorithms,
    /// Specifies the MAC (message authentication code) algorithms in order of preference
    pub mac: Algorithms,
    /// Specifies the port number to connect on the remote host.
    pub port: Option<u16>,
    /// Specifies one or more jump proxies as either \[user@\]host\[:port\] or an ssh URI
    pub proxy_jump: Option<Vec<String>>,
    /// Specifies the signature algorithms that will be used for public key authentication
    pub pubkey_accepted_algorithms: Algorithms,
    /// Specifies whether to try public key authentication using SSH keys
    pub pubkey_authentication: Option<bool>,
    /// Specifies that a TCP port on the remote machine be forwarded over the secure channel
    pub remote_forward: Option<u16>,
    /// Sets a timeout interval in seconds after which if no data has been received from the server, keep alive will be sent
    pub server_alive_interval: Option<Duration>,
    /// Specifies whether to send TCP keepalives to the other side
    pub tcp_keep_alive: Option<bool>,
    #[cfg(target_os = "macos")]
    /// specifies whether the system should search for passphrases in the user's keychain when attempting to use a particular key
    pub use_keychain: Option<bool>,
    /// Specifies the user to log in as.
    pub user: Option<String>,
    /// fields that the parser wasn't able to parse
    pub ignored_fields: HashMap<String, Vec<String>>,
    /// fields that the parser was able to parse but ignored
    pub unsupported_fields: HashMap<String, Vec<String>>,
}

impl HostParams {
    /// Create a new [`HostParams`] object with the [`DefaultAlgorithms`]
    pub fn new(default_algorithms: &DefaultAlgorithms) -> Self {
        Self {
            add_keys_to_agent: None,
            bind_address: None,
            bind_interface: None,
            ca_signature_algorithms: Algorithms::new(&default_algorithms.ca_signature_algorithms),
            certificate_file: None,
            ciphers: Algorithms::new(&default_algorithms.ciphers),
            compression: None,
            connection_attempts: None,
            connect_timeout: None,
            forward_agent: None,
            host_key_algorithms: Algorithms::new(&default_algorithms.host_key_algorithms),
            host_name: None,
            identity_file: None,
            ignore_unknown: None,
            kex_algorithms: Algorithms::new(&default_algorithms.kex_algorithms),
            mac: Algorithms::new(&default_algorithms.mac),
            port: None,
            proxy_jump: None,
            pubkey_accepted_algorithms: Algorithms::new(
                &default_algorithms.pubkey_accepted_algorithms,
            ),
            pubkey_authentication: None,
            remote_forward: None,
            server_alive_interval: None,
            tcp_keep_alive: None,
            #[cfg(target_os = "macos")]
            use_keychain: None,
            user: None,
            ignored_fields: HashMap::new(),
            unsupported_fields: HashMap::new(),
        }
    }

    /// Return whether a certain `param` is in the ignored list
    pub(crate) fn ignored(&self, param: &str) -> bool {
        self.ignore_unknown
            .as_ref()
            .map(|x| x.iter().any(|x| x.as_str() == param))
            .unwrap_or(false)
    }

    /// Given a [`HostParams`] object `b`, it will overwrite all the params from `self` only if they are [`None`]
    pub fn overwrite_if_none(&mut self, b: &Self) {
        self.add_keys_to_agent = self.add_keys_to_agent.or(b.add_keys_to_agent);
        self.bind_address = self.bind_address.clone().or_else(|| b.bind_address.clone());
        self.bind_interface = self
            .bind_interface
            .clone()
            .or_else(|| b.bind_interface.clone());
        self.certificate_file = self
            .certificate_file
            .clone()
            .or_else(|| b.certificate_file.clone());
        self.compression = self.compression.or(b.compression);
        self.connection_attempts = self.connection_attempts.or(b.connection_attempts);
        self.connect_timeout = self.connect_timeout.or(b.connect_timeout);
        self.forward_agent = self.forward_agent.or(b.forward_agent);
        self.host_name = self.host_name.clone().or_else(|| b.host_name.clone());
        // IdentityFile accumulates across Host blocks (unlike other directives)
        match (&mut self.identity_file, &b.identity_file) {
            (Some(existing), Some(other)) => existing.extend(other.clone()),
            (None, Some(other)) => self.identity_file = Some(other.clone()),
            _ => {}
        }
        self.ignore_unknown = self
            .ignore_unknown
            .clone()
            .or_else(|| b.ignore_unknown.clone());
        self.port = self.port.or(b.port);
        self.proxy_jump = self.proxy_jump.clone().or_else(|| b.proxy_jump.clone());
        self.pubkey_authentication = self.pubkey_authentication.or(b.pubkey_authentication);
        self.remote_forward = self.remote_forward.or(b.remote_forward);
        self.server_alive_interval = self.server_alive_interval.or(b.server_alive_interval);
        #[cfg(target_os = "macos")]
        {
            self.use_keychain = self.use_keychain.or(b.use_keychain);
        }
        self.tcp_keep_alive = self.tcp_keep_alive.or(b.tcp_keep_alive);
        self.user = self.user.clone().or_else(|| b.user.clone());
        for (ignored_field, args) in &b.ignored_fields {
            if !self.ignored_fields.contains_key(ignored_field) {
                self.ignored_fields
                    .insert(ignored_field.to_owned(), args.to_owned());
            }
        }
        for (unsupported_field, args) in &b.unsupported_fields {
            if !self.unsupported_fields.contains_key(unsupported_field) {
                self.unsupported_fields
                    .insert(unsupported_field.to_owned(), args.to_owned());
            }
        }

        // merge algos if default and b is not default
        if self.ca_signature_algorithms.is_default() && !b.ca_signature_algorithms.is_default() {
            self.ca_signature_algorithms = b.ca_signature_algorithms.clone();
        }
        if self.ciphers.is_default() && !b.ciphers.is_default() {
            self.ciphers = b.ciphers.clone();
        }
        if self.host_key_algorithms.is_default() && !b.host_key_algorithms.is_default() {
            self.host_key_algorithms = b.host_key_algorithms.clone();
        }
        if self.kex_algorithms.is_default() && !b.kex_algorithms.is_default() {
            self.kex_algorithms = b.kex_algorithms.clone();
        }
        if self.mac.is_default() && !b.mac.is_default() {
            self.mac = b.mac.clone();
        }
        if self.pubkey_accepted_algorithms.is_default()
            && !b.pubkey_accepted_algorithms.is_default()
        {
            self.pubkey_accepted_algorithms = b.pubkey_accepted_algorithms.clone();
        }
    }
}

#[cfg(test)]
mod tests {

    use std::str::FromStr;

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::params::algos::AlgorithmsRule;

    #[test]
    fn should_initialize_params() {
        let params = HostParams::new(&DefaultAlgorithms::default());
        assert!(params.add_keys_to_agent.is_none());
        assert!(params.bind_address.is_none());
        assert!(params.bind_interface.is_none());
        assert_eq!(
            params.ca_signature_algorithms.algorithms(),
            DefaultAlgorithms::default().ca_signature_algorithms
        );
        assert!(params.certificate_file.is_none());
        assert_eq!(
            params.ciphers.algorithms(),
            DefaultAlgorithms::default().ciphers
        );
        assert!(params.compression.is_none());
        assert!(params.connection_attempts.is_none());
        assert!(params.connect_timeout.is_none());
        assert!(params.forward_agent.is_none());
        assert_eq!(
            params.host_key_algorithms.algorithms(),
            DefaultAlgorithms::default().host_key_algorithms
        );
        assert!(params.host_name.is_none());
        assert!(params.identity_file.is_none());
        assert!(params.ignore_unknown.is_none());
        assert_eq!(
            params.kex_algorithms.algorithms(),
            DefaultAlgorithms::default().kex_algorithms
        );
        assert_eq!(params.mac.algorithms(), DefaultAlgorithms::default().mac);
        assert!(params.port.is_none());
        assert!(params.proxy_jump.is_none());
        assert_eq!(
            params.pubkey_accepted_algorithms.algorithms(),
            DefaultAlgorithms::default().pubkey_accepted_algorithms
        );
        assert!(params.pubkey_authentication.is_none());
        assert!(params.remote_forward.is_none());
        assert!(params.server_alive_interval.is_none());
        #[cfg(target_os = "macos")]
        assert!(params.use_keychain.is_none());
        assert!(params.tcp_keep_alive.is_none());
    }

    #[test]
    fn test_should_overwrite_if_none() {
        let mut params = HostParams::new(&DefaultAlgorithms::default());
        params.bind_address = Some(String::from("pippo"));

        let mut b = HostParams::new(&DefaultAlgorithms::default());
        b.bind_address = Some(String::from("pluto"));
        b.bind_interface = Some(String::from("tun0"));
        b.ciphers
            .apply(AlgorithmsRule::from_str("c,d").expect("parse error"));

        params.overwrite_if_none(&b);
        assert_eq!(params.bind_address.unwrap(), "pippo");
        assert_eq!(params.bind_interface.unwrap(), "tun0");

        // algos
        assert_eq!(
            params.ciphers.algorithms(),
            vec!["c".to_string(), "d".to_string()]
        );
    }

    #[test]
    fn test_ignored_returns_false_when_none() {
        let params = HostParams::new(&DefaultAlgorithms::default());
        assert!(!params.ignored("SomeParam"));
    }

    #[test]
    fn test_ignored_returns_false_when_not_in_list() {
        let mut params = HostParams::new(&DefaultAlgorithms::default());
        params.ignore_unknown = Some(vec!["Param1".to_string(), "Param2".to_string()]);
        assert!(!params.ignored("OtherParam"));
    }

    #[test]
    fn test_ignored_returns_true_when_in_list() {
        let mut params = HostParams::new(&DefaultAlgorithms::default());
        params.ignore_unknown = Some(vec!["Param1".to_string(), "Param2".to_string()]);
        assert!(params.ignored("Param1"));
        assert!(params.ignored("Param2"));
    }

    #[test]
    fn test_overwrite_if_none_all_fields() {
        let mut params = HostParams::new(&DefaultAlgorithms::empty());

        let mut b = HostParams::new(&DefaultAlgorithms::empty());
        b.add_keys_to_agent = Some(true);
        b.bind_address = Some(String::from("addr"));
        b.bind_interface = Some(String::from("iface"));
        b.certificate_file = Some(std::path::PathBuf::from("/cert"));
        b.compression = Some(true);
        b.connection_attempts = Some(5);
        b.connect_timeout = Some(Duration::from_secs(30));
        b.forward_agent = Some(true);
        b.host_name = Some(String::from("host"));
        b.identity_file = Some(vec![std::path::PathBuf::from("/id")]);
        b.ignore_unknown = Some(vec!["field".to_string()]);
        b.port = Some(22);
        b.proxy_jump = Some(vec!["proxy".to_string()]);
        b.pubkey_authentication = Some(true);
        b.remote_forward = Some(8080);
        b.server_alive_interval = Some(Duration::from_secs(60));
        b.tcp_keep_alive = Some(true);
        #[cfg(target_os = "macos")]
        {
            b.use_keychain = Some(true);
        }
        b.user = Some(String::from("user"));
        b.ignored_fields
            .insert("custom".to_string(), vec!["value".to_string()]);
        b.unsupported_fields
            .insert("unsupported".to_string(), vec!["val".to_string()]);

        params.overwrite_if_none(&b);

        assert_eq!(params.add_keys_to_agent, Some(true));
        assert_eq!(params.bind_address, Some(String::from("addr")));
        assert_eq!(params.bind_interface, Some(String::from("iface")));
        assert_eq!(
            params.certificate_file,
            Some(std::path::PathBuf::from("/cert"))
        );
        assert_eq!(params.compression, Some(true));
        assert_eq!(params.connection_attempts, Some(5));
        assert_eq!(params.connect_timeout, Some(Duration::from_secs(30)));
        assert_eq!(params.forward_agent, Some(true));
        assert_eq!(params.host_name, Some(String::from("host")));
        assert_eq!(
            params.identity_file,
            Some(vec![std::path::PathBuf::from("/id")])
        );
        assert_eq!(params.ignore_unknown, Some(vec!["field".to_string()]));
        assert_eq!(params.port, Some(22));
        assert_eq!(params.proxy_jump, Some(vec!["proxy".to_string()]));
        assert_eq!(params.pubkey_authentication, Some(true));
        assert_eq!(params.remote_forward, Some(8080));
        assert_eq!(params.server_alive_interval, Some(Duration::from_secs(60)));
        assert_eq!(params.tcp_keep_alive, Some(true));
        #[cfg(target_os = "macos")]
        assert_eq!(params.use_keychain, Some(true));
        assert_eq!(params.user, Some(String::from("user")));
        assert!(params.ignored_fields.contains_key("custom"));
        assert!(params.unsupported_fields.contains_key("unsupported"));
    }

    #[test]
    fn test_overwrite_if_none_does_not_overwrite_existing() {
        let mut params = HostParams::new(&DefaultAlgorithms::empty());
        params.add_keys_to_agent = Some(false);
        params.bind_address = Some(String::from("original"));
        params.compression = Some(false);
        params.port = Some(2222);
        params.user = Some(String::from("original_user"));
        params
            .ignored_fields
            .insert("existing".to_string(), vec!["val1".to_string()]);
        params
            .unsupported_fields
            .insert("existing_unsup".to_string(), vec!["val1".to_string()]);

        let mut b = HostParams::new(&DefaultAlgorithms::empty());
        b.add_keys_to_agent = Some(true);
        b.bind_address = Some(String::from("new"));
        b.compression = Some(true);
        b.port = Some(22);
        b.user = Some(String::from("new_user"));
        b.ignored_fields
            .insert("existing".to_string(), vec!["val2".to_string()]);
        b.unsupported_fields
            .insert("existing_unsup".to_string(), vec!["val2".to_string()]);

        params.overwrite_if_none(&b);

        // Should keep original values
        assert_eq!(params.add_keys_to_agent, Some(false));
        assert_eq!(params.bind_address, Some(String::from("original")));
        assert_eq!(params.compression, Some(false));
        assert_eq!(params.port, Some(2222));
        assert_eq!(params.user, Some(String::from("original_user")));
        assert_eq!(
            params.ignored_fields.get("existing"),
            Some(&vec!["val1".to_string()])
        );
        assert_eq!(
            params.unsupported_fields.get("existing_unsup"),
            Some(&vec!["val1".to_string()])
        );
    }

    #[test]
    fn test_overwrite_if_none_algorithms_when_self_is_default() {
        let mut params = HostParams::new(&DefaultAlgorithms::empty());

        let mut b = HostParams::new(&DefaultAlgorithms::empty());
        b.ca_signature_algorithms
            .apply(AlgorithmsRule::from_str("ca-algo").expect("parse error"));
        b.host_key_algorithms
            .apply(AlgorithmsRule::from_str("hk-algo").expect("parse error"));
        b.kex_algorithms
            .apply(AlgorithmsRule::from_str("kex-algo").expect("parse error"));
        b.mac
            .apply(AlgorithmsRule::from_str("mac-algo").expect("parse error"));
        b.pubkey_accepted_algorithms
            .apply(AlgorithmsRule::from_str("pk-algo").expect("parse error"));

        params.overwrite_if_none(&b);

        assert_eq!(
            params.ca_signature_algorithms.algorithms(),
            &["ca-algo".to_string()]
        );
        assert_eq!(
            params.host_key_algorithms.algorithms(),
            &["hk-algo".to_string()]
        );
        assert_eq!(
            params.kex_algorithms.algorithms(),
            &["kex-algo".to_string()]
        );
        assert_eq!(params.mac.algorithms(), &["mac-algo".to_string()]);
        assert_eq!(
            params.pubkey_accepted_algorithms.algorithms(),
            &["pk-algo".to_string()]
        );
    }

    #[test]
    fn test_overwrite_if_none_algorithms_when_self_is_not_default() {
        let mut params = HostParams::new(&DefaultAlgorithms::empty());
        params
            .ciphers
            .apply(AlgorithmsRule::from_str("self-cipher").expect("parse error"));

        let mut b = HostParams::new(&DefaultAlgorithms::empty());
        b.ciphers
            .apply(AlgorithmsRule::from_str("other-cipher").expect("parse error"));

        params.overwrite_if_none(&b);

        // Self's cipher should remain since it was already overridden
        assert_eq!(params.ciphers.algorithms(), &["self-cipher".to_string()]);
    }

    #[test]
    fn test_overwrite_if_none_accumulates_identity_files() {
        let mut params = HostParams::new(&DefaultAlgorithms::empty());
        params.identity_file = Some(vec![std::path::PathBuf::from("/path/to/key1")]);

        let mut b = HostParams::new(&DefaultAlgorithms::empty());
        b.identity_file = Some(vec![
            std::path::PathBuf::from("/path/to/key2"),
            std::path::PathBuf::from("/path/to/key3"),
        ]);

        params.overwrite_if_none(&b);

        // Identity files should be accumulated, not replaced
        assert_eq!(
            params.identity_file,
            Some(vec![
                std::path::PathBuf::from("/path/to/key1"),
                std::path::PathBuf::from("/path/to/key2"),
                std::path::PathBuf::from("/path/to/key3"),
            ])
        );
    }

    #[test]
    fn test_overwrite_if_none_identity_files_when_self_is_none() {
        let mut params = HostParams::new(&DefaultAlgorithms::empty());

        let mut b = HostParams::new(&DefaultAlgorithms::empty());
        b.identity_file = Some(vec![std::path::PathBuf::from("/path/to/key1")]);

        params.overwrite_if_none(&b);

        assert_eq!(
            params.identity_file,
            Some(vec![std::path::PathBuf::from("/path/to/key1")])
        );
    }
}
