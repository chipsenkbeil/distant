use crate::serde_str::{deserialize_from_str, serialize_to_str};
use derive_more::{Display, Error, From};
use serde::{de::Deserializer, ser::Serializer, Deserialize, Serialize};
use std::{
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    str::FromStr,
};

/// Represents the host of a destination
#[derive(Clone, Debug, From, Display, Hash, PartialEq, Eq)]
pub enum Host {
    Ipv4(Ipv4Addr),
    Ipv6(Ipv6Addr),

    /// Represents a hostname that follows the
    /// [DoD Internet Host Table Specification](https://www.ietf.org/rfc/rfc0952.txt):
    ///
    /// * Hostname can be a maximum of 253 characters including '.'
    /// * Each label is a-zA-Z0-9 alongside hyphen ('-') and a maximum size of 63 characters
    /// * Labels can be segmented by periods ('.')
    Name(String),
}

impl Host {
    /// Indicates whether the host destination is globally routable
    pub const fn is_global(&self) -> bool {
        match self {
            Self::Ipv4(x) => {
                !(x.is_broadcast()
                    || x.is_documentation()
                    || x.is_link_local()
                    || x.is_loopback()
                    || x.is_private()
                    || x.is_unspecified())
            }
            Self::Ipv6(x) => {
                // NOTE: 14 is the global flag
                x.is_multicast() && (x.segments()[0] & 0x000f == 14)
            }
            Self::Name(_) => false,
        }
    }

    /// Returns true if host is an IPv4 address
    pub const fn is_ipv4(&self) -> bool {
        matches!(self, Self::Ipv4(_))
    }

    /// Returns true if host is an IPv6 address
    pub const fn is_ipv6(&self) -> bool {
        matches!(self, Self::Ipv6(_))
    }

    /// Returns true if host is a name
    pub const fn is_name(&self) -> bool {
        matches!(self, Self::Name(_))
    }
}

impl From<IpAddr> for Host {
    fn from(addr: IpAddr) -> Self {
        match addr {
            IpAddr::V4(x) => Self::Ipv4(x),
            IpAddr::V6(x) => Self::Ipv6(x),
        }
    }
}

#[derive(Copy, Clone, Debug, Error, Hash, PartialEq, Eq)]
pub enum HostParseError {
    EmptyLabel,
    EndsWithHyphen,
    EndsWithPeriod,
    InvalidLabel,
    LargeLabel,
    LargeName,
    StartsWithHyphen,
    StartsWithPeriod,
}

impl HostParseError {
    /// Returns a static `str` describing the error
    pub const fn into_static_str(self) -> &'static str {
        match self {
            Self::EmptyLabel => "Hostname cannot have an empty label",
            Self::EndsWithHyphen => "Hostname cannot end with hyphen ('-')",
            Self::EndsWithPeriod => "Hostname cannot end with period ('.')",
            Self::InvalidLabel => "Hostname label can only be a-zA-Z0-9 or hyphen ('-')",
            Self::LargeLabel => "Hostname label larger cannot be larger than 63 characters",
            Self::LargeName => "Hostname cannot be larger than 253 characters",
            Self::StartsWithHyphen => "Hostname cannot start with hyphen ('-')",
            Self::StartsWithPeriod => "Hostname cannot start with period ('.')",
        }
    }
}

impl fmt::Display for HostParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.into_static_str())
    }
}

impl FromStr for Host {
    type Err = HostParseError;

    /// Parses a host from a str
    ///
    /// ### Examples
    ///
    /// ```
    /// # use distant_core::Host;
    /// # use std::net::{Ipv4Addr, Ipv6Addr};
    /// // IPv4 address
    /// assert_eq!("127.0.0.1".parse(), Ok(Host::Ipv4(Ipv4Addr::new(127, 0, 0, 1))));
    ///
    /// // IPv6 address
    /// assert_eq!("::1".parse(), Ok(Host::Ipv6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1))));
    ///
    /// // Valid hostname
    /// assert_eq!("localhost".parse(), Ok(Host::Name("localhost".to_string())));
    ///
    /// // Invalid hostname
    /// assert!("local_host".parse::<Host>().is_err());
    /// ```
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Check if the str is a valid Ipv4 or Ipv6 address first
        if let Ok(x) = s.parse::<Ipv4Addr>() {
            return Ok(Self::Ipv4(x));
        } else if let Ok(x) = s.parse::<Ipv6Addr>() {
            return Ok(Self::Ipv6(x));
        }

        // NOTE: We have to catch an empty string seprately from invalid label checks
        if s.is_empty() {
            return Err(HostParseError::InvalidLabel);
        }

        // Since it is not, we need to validate the string as a hostname
        let mut label_size_cnt = 0;
        let mut last_char = None;
        for (i, c) in s.char_indices() {
            if i >= 253 {
                return Err(HostParseError::LargeName);
            }

            // Dot and hyphen cannot be first character
            if i == 0 && c == '.' {
                return Err(HostParseError::StartsWithPeriod);
            } else if i == 0 && c == '-' {
                return Err(HostParseError::StartsWithHyphen);
            }

            if c.is_alphanumeric() {
                label_size_cnt += 1;
                if label_size_cnt > 63 {
                    return Err(HostParseError::LargeLabel);
                }
            } else if c == '.' {
                // Back-to-back dots are not allowed (would indicate an empty label, which is
                // reserved)
                if label_size_cnt == 0 {
                    return Err(HostParseError::EmptyLabel);
                }

                label_size_cnt = 0;
            } else if c != '-' {
                return Err(HostParseError::InvalidLabel);
            }

            last_char = Some(c);
        }

        if last_char == Some('.') {
            return Err(HostParseError::EndsWithPeriod);
        } else if last_char == Some('-') {
            return Err(HostParseError::EndsWithHyphen);
        }

        Ok(Self::Name(s.to_string()))
    }
}

impl PartialEq<str> for Host {
    fn eq(&self, other: &str) -> bool {
        match self {
            Self::Ipv4(x) => x.to_string() == other,
            Self::Ipv6(x) => x.to_string() == other,
            Self::Name(x) => x == other,
        }
    }
}

impl<'a> PartialEq<&'a str> for Host {
    fn eq(&self, other: &&'a str) -> bool {
        match self {
            Self::Ipv4(x) => x.to_string() == *other,
            Self::Ipv6(x) => x.to_string() == *other,
            Self::Name(x) => x == other,
        }
    }
}

impl Serialize for Host {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_to_str(self, serializer)
    }
}

impl<'de> Deserialize<'de> for Host {
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
    fn display_should_output_ipv4_correctly() {
        let host = Host::Ipv4(Ipv4Addr::LOCALHOST);
        assert_eq!(host.to_string(), "127.0.0.1");
    }

    #[test]
    fn display_should_output_ipv6_correctly() {
        let host = Host::Ipv6(Ipv6Addr::LOCALHOST);
        assert_eq!(host.to_string(), "::1");
    }

    #[test]
    fn display_should_output_hostname_verbatim() {
        let host = Host::Name("localhost".to_string());
        assert_eq!(host.to_string(), "localhost");
    }

    #[test]
    fn from_str_should_fail_if_str_is_empty() {
        let err = "".parse::<Host>().unwrap_err();
        assert_eq!(err, HostParseError::InvalidLabel);
    }

    #[test]
    fn from_str_should_fail_if_str_is_larger_than_253_characters() {
        // 63 + 1 + 63 + 1 + 63 + 1 + 62 = 254 characters
        let long_name = format!(
            "{}.{}.{}.{}",
            "a".repeat(63),
            "a".repeat(63),
            "a".repeat(63),
            "a".repeat(62)
        );
        let err = long_name.parse::<Host>().unwrap_err();
        assert_eq!(err, HostParseError::LargeName);
    }

    #[test]
    fn from_str_should_fail_if_str_starts_with_period() {
        let err = ".localhost".parse::<Host>().unwrap_err();
        assert_eq!(err, HostParseError::StartsWithPeriod);
    }

    #[test]
    fn from_str_should_fail_if_str_ends_with_period() {
        let err = "localhost.".parse::<Host>().unwrap_err();
        assert_eq!(err, HostParseError::EndsWithPeriod);
    }

    #[test]
    fn from_str_should_fail_if_str_starts_with_hyphen() {
        let err = "-localhost".parse::<Host>().unwrap_err();
        assert_eq!(err, HostParseError::StartsWithHyphen);
    }

    #[test]
    fn from_str_should_fail_if_str_ends_with_hyphen() {
        let err = "localhost-".parse::<Host>().unwrap_err();
        assert_eq!(err, HostParseError::EndsWithHyphen);
    }

    #[test]
    fn from_str_should_fail_if_str_has_a_label_larger_than_63_characters() {
        let long_label = format!("{}.com", "a".repeat(64));
        let err = long_label.parse::<Host>().unwrap_err();
        assert_eq!(err, HostParseError::LargeLabel);
    }

    #[test]
    fn from_str_should_fail_if_str_has_empty_label() {
        let err = "example..com".parse::<Host>().unwrap_err();
        assert_eq!(err, HostParseError::EmptyLabel);
    }

    #[test]
    fn from_str_should_fail_if_str_has_invalid_label() {
        let err = "www.exa_mple.com".parse::<Host>().unwrap_err();
        assert_eq!(err, HostParseError::InvalidLabel);
    }

    #[test]
    fn from_str_should_succeed_if_valid_ipv4_address() {
        let host = "127.0.0.1".parse::<Host>().unwrap();
        assert_eq!(host, Host::Ipv4(Ipv4Addr::new(127, 0, 0, 1)));
    }

    #[test]
    fn from_str_should_succeed_if_valid_ipv6_address() {
        let host = "::1".parse::<Host>().unwrap();
        assert_eq!(host, Host::Ipv6(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1)));
    }

    #[test]
    fn from_str_should_succeed_if_valid_hostname() {
        let host = "localhost".parse::<Host>().unwrap();
        assert_eq!(host, Host::Name("localhost".to_string()));

        let host = "example.com".parse::<Host>().unwrap();
        assert_eq!(host, Host::Name("example.com".to_string()));

        let host = "w-w-w.example.com".parse::<Host>().unwrap();
        assert_eq!(host, Host::Name("w-w-w.example.com".to_string()));

        let host = "w3.example.com".parse::<Host>().unwrap();
        assert_eq!(host, Host::Name("w3.example.com".to_string()));

        // Revision of RFC-952 via RFC-1123 allows digit at start of label
        let host = "3.example.com".parse::<Host>().unwrap();
        assert_eq!(host, Host::Name("3.example.com".to_string()));
    }
}
