use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use derive_more::{Display, Error, From};
use serde::de::Deserializer;
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

use super::{deserialize_from_str, serialize_to_str};

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
    /// # use distant_core::net::common::Host;
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
    //! Tests for Host enum: is_global classification, type predicates, From conversions,
    //! PartialEq<str>, serde round-trips, hostname boundary validation, HostParseError
    //! Display, and Hash/Clone.

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

    // -----------------------------------------------------------------------
    // is_global() for IPv4 variants
    // -----------------------------------------------------------------------

    #[test]
    fn is_global_returns_false_for_ipv4_loopback() {
        let host = Host::Ipv4(Ipv4Addr::LOCALHOST);
        assert!(!host.is_global());
    }

    #[test]
    fn is_global_returns_false_for_ipv4_private() {
        // 10.0.0.1 is private
        let host = Host::Ipv4(Ipv4Addr::new(10, 0, 0, 1));
        assert!(!host.is_global());

        // 172.16.0.1 is private
        let host = Host::Ipv4(Ipv4Addr::new(172, 16, 0, 1));
        assert!(!host.is_global());

        // 192.168.1.1 is private
        let host = Host::Ipv4(Ipv4Addr::new(192, 168, 1, 1));
        assert!(!host.is_global());
    }

    #[test]
    fn is_global_returns_false_for_ipv4_broadcast() {
        let host = Host::Ipv4(Ipv4Addr::BROADCAST);
        assert!(!host.is_global());
    }

    #[test]
    fn is_global_returns_false_for_ipv4_unspecified() {
        let host = Host::Ipv4(Ipv4Addr::UNSPECIFIED);
        assert!(!host.is_global());
    }

    #[test]
    fn is_global_returns_false_for_ipv4_link_local() {
        // 169.254.x.x is link-local
        let host = Host::Ipv4(Ipv4Addr::new(169, 254, 0, 1));
        assert!(!host.is_global());
    }

    #[test]
    fn is_global_returns_false_for_ipv4_documentation() {
        // 192.0.2.1 is documentation (TEST-NET-1)
        let host = Host::Ipv4(Ipv4Addr::new(192, 0, 2, 1));
        assert!(!host.is_global());
    }

    #[test]
    fn is_global_returns_true_for_ipv4_public_address() {
        // 8.8.8.8 is a public address
        let host = Host::Ipv4(Ipv4Addr::new(8, 8, 8, 8));
        assert!(host.is_global());
    }

    // -----------------------------------------------------------------------
    // is_global() for IPv6 variants
    // -----------------------------------------------------------------------

    #[test]
    fn is_global_returns_false_for_ipv6_loopback() {
        let host = Host::Ipv6(Ipv6Addr::LOCALHOST);
        assert!(!host.is_global());
    }

    #[test]
    fn is_global_returns_false_for_ipv6_non_multicast() {
        // A regular (non-multicast) IPv6 address returns false because the
        // is_global logic only checks multicast addresses with global flag
        let host = Host::Ipv6(Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1));
        assert!(!host.is_global());
    }

    #[test]
    fn is_global_returns_true_for_ipv6_multicast_global() {
        // ff0e::1 is multicast with global scope (flag 14 = 0xe)
        let host = Host::Ipv6(Ipv6Addr::new(0xff0e, 0, 0, 0, 0, 0, 0, 1));
        assert!(host.is_global());
    }

    #[test]
    fn is_global_returns_false_for_ipv6_multicast_link_local() {
        // ff02::1 is multicast with link-local scope (flag 2)
        let host = Host::Ipv6(Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 1));
        assert!(!host.is_global());
    }

    // -----------------------------------------------------------------------
    // is_global() for Name variant
    // -----------------------------------------------------------------------

    #[test]
    fn is_global_returns_false_for_name() {
        let host = Host::Name("example.com".to_string());
        assert!(!host.is_global());
    }

    // -----------------------------------------------------------------------
    // is_ipv4 / is_ipv6 / is_name predicates
    // -----------------------------------------------------------------------

    #[test]
    fn is_ipv4_returns_true_only_for_ipv4() {
        assert!(Host::Ipv4(Ipv4Addr::LOCALHOST).is_ipv4());
        assert!(!Host::Ipv6(Ipv6Addr::LOCALHOST).is_ipv4());
        assert!(!Host::Name("localhost".to_string()).is_ipv4());
    }

    #[test]
    fn is_ipv6_returns_true_only_for_ipv6() {
        assert!(!Host::Ipv4(Ipv4Addr::LOCALHOST).is_ipv6());
        assert!(Host::Ipv6(Ipv6Addr::LOCALHOST).is_ipv6());
        assert!(!Host::Name("localhost".to_string()).is_ipv6());
    }

    #[test]
    fn is_name_returns_true_only_for_name() {
        assert!(!Host::Ipv4(Ipv4Addr::LOCALHOST).is_name());
        assert!(!Host::Ipv6(Ipv6Addr::LOCALHOST).is_name());
        assert!(Host::Name("localhost".to_string()).is_name());
    }

    // -----------------------------------------------------------------------
    // HostParseError display messages
    // -----------------------------------------------------------------------

    #[test]
    fn host_parse_error_display_covers_all_variants() {
        assert_eq!(
            HostParseError::EmptyLabel.to_string(),
            "Hostname cannot have an empty label"
        );
        assert_eq!(
            HostParseError::EndsWithHyphen.to_string(),
            "Hostname cannot end with hyphen ('-')"
        );
        assert_eq!(
            HostParseError::EndsWithPeriod.to_string(),
            "Hostname cannot end with period ('.')"
        );
        assert_eq!(
            HostParseError::InvalidLabel.to_string(),
            "Hostname label can only be a-zA-Z0-9 or hyphen ('-')"
        );
        assert_eq!(
            HostParseError::LargeLabel.to_string(),
            "Hostname label larger cannot be larger than 63 characters"
        );
        assert_eq!(
            HostParseError::LargeName.to_string(),
            "Hostname cannot be larger than 253 characters"
        );
        assert_eq!(
            HostParseError::StartsWithHyphen.to_string(),
            "Hostname cannot start with hyphen ('-')"
        );
        assert_eq!(
            HostParseError::StartsWithPeriod.to_string(),
            "Hostname cannot start with period ('.')"
        );
    }

    #[test]
    fn host_parse_error_into_static_str_matches_display() {
        let variants = [
            HostParseError::EmptyLabel,
            HostParseError::EndsWithHyphen,
            HostParseError::EndsWithPeriod,
            HostParseError::InvalidLabel,
            HostParseError::LargeLabel,
            HostParseError::LargeName,
            HostParseError::StartsWithHyphen,
            HostParseError::StartsWithPeriod,
        ];
        for variant in variants {
            assert_eq!(variant.into_static_str(), variant.to_string());
        }
    }

    // -----------------------------------------------------------------------
    // From<IpAddr> impl
    // -----------------------------------------------------------------------

    #[test]
    fn from_ipaddr_v4_creates_ipv4_host() {
        let addr = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let host = Host::from(addr);
        assert!(host.is_ipv4());
        assert_eq!(host, Host::Ipv4(Ipv4Addr::new(10, 0, 0, 1)));
    }

    #[test]
    fn from_ipaddr_v6_creates_ipv6_host() {
        let addr = IpAddr::V6(Ipv6Addr::LOCALHOST);
        let host = Host::from(addr);
        assert!(host.is_ipv6());
        assert_eq!(host, Host::Ipv6(Ipv6Addr::LOCALHOST));
    }

    #[test]
    fn from_ipv4addr_directly_creates_ipv4_host() {
        let host = Host::from(Ipv4Addr::new(1, 2, 3, 4));
        assert_eq!(host, Host::Ipv4(Ipv4Addr::new(1, 2, 3, 4)));
    }

    #[test]
    fn from_ipv6addr_directly_creates_ipv6_host() {
        let host = Host::from(Ipv6Addr::new(0, 0, 0, 0, 0, 0, 0, 1));
        assert_eq!(host, Host::Ipv6(Ipv6Addr::LOCALHOST));
    }

    // -----------------------------------------------------------------------
    // PartialEq<str> and PartialEq<&str>
    // -----------------------------------------------------------------------

    #[test]
    fn partial_eq_str_works_for_ipv4() {
        let host = Host::Ipv4(Ipv4Addr::new(127, 0, 0, 1));
        assert_eq!(host, *"127.0.0.1");
        assert_ne!(host, *"127.0.0.2");
    }

    #[test]
    fn partial_eq_str_works_for_ipv6() {
        let host = Host::Ipv6(Ipv6Addr::LOCALHOST);
        assert_eq!(host, *"::1");
    }

    #[test]
    fn partial_eq_str_works_for_name() {
        let host = Host::Name("example.com".to_string());
        assert_eq!(host, *"example.com");
        assert_ne!(host, *"other.com");
    }

    #[test]
    fn partial_eq_ref_str_works_for_all_variants() {
        let ipv4 = Host::Ipv4(Ipv4Addr::LOCALHOST);
        assert_eq!(ipv4, "127.0.0.1");

        let ipv6 = Host::Ipv6(Ipv6Addr::LOCALHOST);
        assert_eq!(ipv6, "::1");

        let name = Host::Name("host".to_string());
        assert_eq!(name, "host");
    }

    // -----------------------------------------------------------------------
    // Serde round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn serde_roundtrip_ipv4() {
        let original = Host::Ipv4(Ipv4Addr::new(192, 168, 1, 1));
        let json = serde_json::to_string(&original).unwrap();
        let restored: Host = serde_json::from_str(&json).unwrap();
        assert_eq!(original, restored);
    }

    #[test]
    fn serde_roundtrip_ipv6() {
        let original = Host::Ipv6(Ipv6Addr::LOCALHOST);
        let json = serde_json::to_string(&original).unwrap();
        let restored: Host = serde_json::from_str(&json).unwrap();
        assert_eq!(original, restored);
    }

    #[test]
    fn serde_roundtrip_name() {
        let original = Host::Name("my-host.example.com".to_string());
        let json = serde_json::to_string(&original).unwrap();
        let restored: Host = serde_json::from_str(&json).unwrap();
        assert_eq!(original, restored);
    }

    // -----------------------------------------------------------------------
    // Boundary tests for hostname lengths
    // -----------------------------------------------------------------------

    #[test]
    fn from_str_should_succeed_for_exactly_253_character_hostname() {
        // 63 + 1 + 63 + 1 + 63 + 1 + 61 = 253 characters (just within limit)
        let name = format!(
            "{}.{}.{}.{}",
            "a".repeat(63),
            "b".repeat(63),
            "c".repeat(63),
            "d".repeat(61)
        );
        assert_eq!(name.len(), 253);
        let host = name.parse::<Host>().unwrap();
        assert!(host.is_name());
    }

    #[test]
    fn from_str_should_succeed_for_exactly_63_char_label() {
        let name = "a".repeat(63);
        let host = name.parse::<Host>().unwrap();
        assert_eq!(host, Host::Name(name));
    }

    #[test]
    fn from_str_should_fail_for_64_char_label() {
        let name = "a".repeat(64);
        let err = name.parse::<Host>().unwrap_err();
        assert_eq!(err, HostParseError::LargeLabel);
    }

    #[test]
    fn from_str_should_succeed_for_single_char_hostname() {
        let host = "a".parse::<Host>().unwrap();
        assert_eq!(host, Host::Name("a".to_string()));
    }

    #[test]
    fn from_str_should_accept_hyphen_in_middle_of_label() {
        let host = "my-host".parse::<Host>().unwrap();
        assert_eq!(host, Host::Name("my-host".to_string()));
    }

    #[test]
    fn from_str_should_accept_multiple_segments() {
        let host = "a.b.c.d.e".parse::<Host>().unwrap();
        assert_eq!(host, Host::Name("a.b.c.d.e".to_string()));
    }

    // -----------------------------------------------------------------------
    // Hash impl (via derive)
    // -----------------------------------------------------------------------

    #[test]
    fn equal_hosts_have_equal_hashes() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        fn hash_of(h: &Host) -> u64 {
            let mut hasher = DefaultHasher::new();
            h.hash(&mut hasher);
            hasher.finish()
        }

        let a = Host::Ipv4(Ipv4Addr::LOCALHOST);
        let b = Host::Ipv4(Ipv4Addr::LOCALHOST);
        assert_eq!(hash_of(&a), hash_of(&b));

        let c = Host::Name("test".to_string());
        let d = Host::Name("test".to_string());
        assert_eq!(hash_of(&c), hash_of(&d));
    }

    // -----------------------------------------------------------------------
    // Clone impl (via derive)
    // -----------------------------------------------------------------------

    #[test]
    fn clone_produces_equal_host() {
        let original = Host::Name("cloneable.com".to_string());
        let cloned = original.clone();
        assert_eq!(original, cloned);
    }
}
