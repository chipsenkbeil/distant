use std::fmt;
use std::hash::Hash;
use std::str::FromStr;

use serde::de::Deserializer;
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

use super::utils::{deserialize_from_str, serialize_to_str};

mod host;
mod parser;

pub use host::{Host, HostParseError};

/// `distant` connects and logs into the specified destination, which may be specified as either
/// `hostname:port` where an attempt to connect to a **distant** server will be made, or a URI of
/// one of the following forms:
///
/// * `distant://hostname:port` - connect to a distant server
/// * `ssh://[user@]hostname[:port]` - connect to an SSH server
///
/// **Note:** Due to the limitations of a URI, an IPv6 address is not supported.
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct Destination {
    /// Sequence of characters beginning with a letter and followed by any combination of letters,
    /// digits, plus (+), period (.), or hyphen (-) representing a scheme associated with a
    /// destination
    pub scheme: Option<String>,

    /// Sequence of alphanumeric characters representing a username tied to a destination
    pub username: Option<String>,

    /// Sequence of alphanumeric characters representing a password tied to a destination
    pub password: Option<String>,

    /// Consisting of either a registered name (including but not limited to a hostname) or an IP
    /// address. IPv4 addresses must be in dot-decimal notation, and IPv6 addresses must be
    /// enclosed in brackets ([])
    pub host: Host,

    /// Port tied to a destination
    pub port: Option<u16>,
}

impl Destination {
    /// Returns true if the destination's scheme represents the specified (case-insensitive).
    pub fn scheme_eq(&self, s: &str) -> bool {
        match self.scheme.as_ref() {
            Some(scheme) => scheme.eq_ignore_ascii_case(s),
            None => false,
        }
    }
}

/// Extracts the scheme from a destination string, if present.
///
/// Returns `None` for bare hosts like `"host:22"`, `Some("ssh")` for `"ssh://host"`.
///
/// # Examples
///
/// ```
/// use distant_core::net::common::extract_scheme;
///
/// assert_eq!(extract_scheme("ssh://host:22"), Some("ssh"));
/// assert_eq!(extract_scheme("docker://ubuntu:22.04"), Some("docker"));
/// assert_eq!(extract_scheme("host:22"), None);
/// ```
pub fn extract_scheme(s: &str) -> Option<&str> {
    s.split_once("://").map(|(scheme, _)| scheme)
}

/// Prepends a default scheme if the destination string has none.
///
/// # Examples
///
/// ```
/// use distant_core::net::common::ensure_scheme;
///
/// assert_eq!(ensure_scheme("host:22", "ssh"), "ssh://host:22");
/// assert_eq!(ensure_scheme("ssh://host:22", "ssh"), "ssh://host:22");
/// ```
pub fn ensure_scheme(s: &str, default: &str) -> String {
    if s.contains("://") {
        s.to_string()
    } else {
        format!("{default}://{s}")
    }
}

impl AsRef<Destination> for &Destination {
    fn as_ref(&self) -> &Destination {
        self
    }
}

impl AsMut<Destination> for &mut Destination {
    fn as_mut(&mut self) -> &mut Destination {
        self
    }
}

impl fmt::Display for Destination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(scheme) = self.scheme.as_ref() {
            write!(f, "{scheme}://")?;
        }

        if let Some(username) = self.username.as_ref() {
            write!(f, "{username}")?;
        }

        if let Some(password) = self.password.as_ref() {
            write!(f, ":{password}")?;
        }

        if self.username.is_some() || self.password.is_some() {
            write!(f, "@")?;
        }

        // For host, if we have a port and are IPv6, we need to wrap in [{}]
        match &self.host {
            Host::Ipv6(x) if self.port.is_some() => write!(f, "[{x}]")?,
            x => write!(f, "{x}")?,
        }

        if let Some(port) = self.port {
            write!(f, ":{port}")?;
        }

        Ok(())
    }
}

impl FromStr for Destination {
    type Err = &'static str;

    /// Parses a destination in the form `[scheme://][[username][:password]@]host[:port]`
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        parser::parse(s)
    }
}

impl FromStr for Box<Destination> {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let destination = s.parse::<Destination>()?;
        Ok(Box::new(destination))
    }
}

impl<'a> PartialEq<&'a str> for Destination {
    #[allow(clippy::cmp_owned)]
    fn eq(&self, other: &&'a str) -> bool {
        self.to_string() == *other
    }
}

impl Serialize for Destination {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_to_str(self, serializer)
    }
}

impl<'de> Deserialize<'de> for Destination {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_from_str(deserializer)
    }
}

#[cfg(test)]
mod tests {
    //! Tests for Destination: scheme_eq case-insensitivity, AsRef/AsMut, Display formatting
    //! with credentials, FromStr for Box<Destination>, and serde round-trips.

    use super::*;

    #[test]
    fn scheme_eq_matches_case_insensitively() {
        let dest = Destination {
            scheme: Some("SSH".to_string()),
            username: None,
            password: None,
            host: Host::Name("host".to_string()),
            port: None,
        };
        assert!(dest.scheme_eq("ssh"));
        assert!(dest.scheme_eq("SSH"));
        assert!(dest.scheme_eq("Ssh"));
        assert!(!dest.scheme_eq("http"));
    }

    #[test]
    fn scheme_eq_returns_false_when_no_scheme() {
        let dest = Destination {
            scheme: None,
            username: None,
            password: None,
            host: Host::Name("host".to_string()),
            port: None,
        };
        assert!(!dest.scheme_eq("ssh"));
    }

    #[test]
    fn as_ref_returns_self() {
        let dest = Destination {
            scheme: None,
            username: None,
            password: None,
            host: Host::Name("host".to_string()),
            port: None,
        };
        // Exercise AsRef<Destination> for &Destination through a generic function
        fn check_as_ref(r: impl AsRef<Destination>) {
            assert_eq!(r.as_ref().host, Host::Name("host".to_string()));
        }
        check_as_ref(&dest);
    }

    #[test]
    fn as_mut_returns_self() {
        let mut dest = Destination {
            scheme: None,
            username: None,
            password: None,
            host: Host::Name("host".to_string()),
            port: None,
        };
        // Exercise AsMut<Destination> for &mut Destination through a generic function
        fn check_as_mut(mut r: impl AsMut<Destination>) {
            r.as_mut().port = Some(22);
        }
        check_as_mut(&mut dest);
        assert_eq!(dest.port, Some(22));
    }

    #[test]
    fn display_with_username_and_password() {
        let dest = Destination {
            scheme: Some("ssh".to_string()),
            username: Some("user".to_string()),
            password: Some("pass".to_string()),
            host: Host::Name("example.com".to_string()),
            port: Some(22),
        };
        assert_eq!(dest.to_string(), "ssh://user:pass@example.com:22");
    }

    #[test]
    fn display_with_username_only() {
        let dest = Destination {
            scheme: Some("ssh".to_string()),
            username: Some("user".to_string()),
            password: None,
            host: Host::Name("example.com".to_string()),
            port: None,
        };
        assert_eq!(dest.to_string(), "ssh://user@example.com");
    }

    #[test]
    fn from_str_for_box_destination() {
        let boxed: Box<Destination> = "ssh://host:22".parse().unwrap();
        assert_eq!(boxed.scheme.as_deref(), Some("ssh"));
        assert_eq!(boxed.host, Host::Name("host".to_string()));
        assert_eq!(boxed.port, Some(22));
    }

    #[test]
    fn serde_round_trip() {
        let dest: Destination = "ssh://user@host:22".parse().unwrap();
        let json = serde_json::to_string(&dest).unwrap();
        let restored: Destination = serde_json::from_str(&json).unwrap();
        assert_eq!(dest, restored);
    }

    #[test]
    fn display_should_output_using_available_components() {
        let destination = Destination {
            scheme: None,
            username: None,
            password: None,
            host: Host::Name("example.com".to_string()),
            port: None,
        };
        assert_eq!(destination, "example.com");
    }

    #[test]
    fn display_should_not_wrap_ipv6_in_square_brackets_if_has_no_port() {
        let destination = Destination {
            scheme: None,
            username: None,
            password: None,
            host: Host::Ipv6("::1".parse().unwrap()),
            port: None,
        };
        assert_eq!(destination, "::1");
    }

    #[test]
    fn display_should_wrap_ipv6_in_square_brackets_if_has_port() {
        let destination = Destination {
            scheme: None,
            username: None,
            password: None,
            host: Host::Ipv6("::1".parse().unwrap()),
            port: Some(12345),
        };
        assert_eq!(destination, "[::1]:12345");
    }

    #[test]
    fn extract_scheme_returns_scheme_when_present() {
        assert_eq!(extract_scheme("ssh://host:22"), Some("ssh"));
        assert_eq!(extract_scheme("docker://ubuntu:22.04"), Some("docker"));
        assert_eq!(extract_scheme("distant://localhost:59699"), Some("distant"));
    }

    #[test]
    fn extract_scheme_returns_none_for_bare_host() {
        assert_eq!(extract_scheme("host:22"), None);
        assert_eq!(extract_scheme("localhost"), None);
    }

    #[test]
    fn ensure_scheme_adds_default_when_missing() {
        assert_eq!(ensure_scheme("host:22", "ssh"), "ssh://host:22");
        assert_eq!(ensure_scheme("localhost", "ssh"), "ssh://localhost");
    }

    #[test]
    fn ensure_scheme_preserves_existing_scheme() {
        assert_eq!(ensure_scheme("ssh://host:22", "docker"), "ssh://host:22");
        assert_eq!(
            ensure_scheme("docker://ubuntu:22.04", "ssh"),
            "docker://ubuntu:22.04"
        );
    }
}
