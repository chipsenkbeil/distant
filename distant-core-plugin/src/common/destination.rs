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
    use super::*;

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
}
