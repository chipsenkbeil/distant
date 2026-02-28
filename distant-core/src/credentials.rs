use std::convert::TryFrom;
use std::str::FromStr;
use std::{fmt, io};

use crate::net::common::{Destination, Host, SecretKey32};
use serde::de::Deserializer;
use serde::ser::Serializer;
use serde::{Deserialize, Serialize};

use crate::serde_str::{deserialize_from_str, serialize_to_str};

const SCHEME: &str = "distant";
const SCHEME_WITH_SEP: &str = "distant://";

/// Represents credentials used for a distant server that is maintaining a single key
/// across all connections
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credentials {
    pub host: Host,
    pub port: u16,
    pub key: SecretKey32,
    pub username: Option<String>,
}

impl fmt::Display for Credentials {
    /// Converts credentials into string in the form of `distant://[username]:{key}@{host}:{port}`
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{SCHEME}://")?;

        if let Some(username) = self.username.as_ref() {
            write!(f, "{username}")?;
        }

        write!(f, ":{}@", self.key)?;

        // If we are IPv6, we need to include square brackets
        if self.host.is_ipv6() {
            write!(f, "[{}]", self.host)?;
        } else {
            write!(f, "{}", self.host)?;
        }

        write!(f, ":{}", self.port)
    }
}

impl FromStr for Credentials {
    type Err = io::Error;

    /// Parse `distant://[username]:{key}@{host}:{port}` as credentials. Note that this requires the
    /// `distant` scheme to be included. If parsing without scheme is desired, call the
    /// [`Credentials::try_from_uri_ref`] method instead with `require_scheme`
    /// set to false
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let destination: Destination = s
            .parse()
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;

        // Can be scheme-less or explicitly distant
        if let Some(scheme) = destination.scheme.as_deref()
            && scheme != SCHEME
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Unexpected scheme: {scheme}"),
            ));
        }

        Ok(Self {
            host: destination.host,
            port: destination
                .port
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Missing port"))?,
            key: destination
                .password
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Missing key"))?
                .parse()
                .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?,
            username: destination.username,
        })
    }
}

impl Serialize for Credentials {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_to_str(self, serializer)
    }
}

impl<'de> Deserialize<'de> for Credentials {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_from_str(deserializer)
    }
}

impl Credentials {
    /// Searches a str for `distant://[username]:{key}@{host}:{port}`, returning the first matching
    /// credentials set if found, failing if anything is found immediately before or after the
    /// credentials that is not whitespace or control characters
    ///
    /// If `strict` is false, then the scheme can be preceded by any character
    pub fn find(s: &str, strict: bool) -> Option<Credentials> {
        let is_boundary = |c| char::is_whitespace(c) || char::is_control(c);

        for (i, _) in s.match_indices(SCHEME_WITH_SEP) {
            // Start at the scheme
            let (before, s) = s.split_at(i);

            // Check character preceding the scheme to make sure it isn't a different scheme
            // Only whitespace or control characters preceding are okay, anything else is skipped
            if strict && !before.is_empty() && !before.ends_with(is_boundary) {
                continue;
            }

            // Consume until we reach whitespace or control, which indicates the potential end
            let s = match s.find(is_boundary) {
                Some(i) => &s[..i],
                None => s,
            };

            match s.parse::<Self>() {
                Ok(this) => return Some(this),
                Err(_) => continue,
            }
        }

        None
    }

    /// Equivalent to [`find(s, true)`].
    ///
    /// [`find(s, true)`]: Credentials::find
    #[inline]
    pub fn find_strict(s: &str) -> Option<Credentials> {
        Self::find(s, true)
    }

    /// Equivalent to [`find(s, false)`].
    ///
    /// [`find(s, false)`]: Credentials::find
    #[inline]
    pub fn find_lax(s: &str) -> Option<Credentials> {
        Self::find(s, false)
    }

    /// Converts credentials into a [`Destination`] of the form
    /// `distant://[username]:{key}@{host}:{port}`, failing if the credentials would not produce a
    /// valid [`Destination`]
    pub fn try_to_destination(&self) -> io::Result<Destination> {
        TryFrom::try_from(self.clone())
    }
}

impl TryFrom<Credentials> for Destination {
    type Error = io::Error;

    fn try_from(credentials: Credentials) -> Result<Self, Self::Error> {
        Ok(Destination {
            scheme: Some("distant".to_string()),
            username: credentials.username,
            password: Some(credentials.key.to_string()),
            host: credentials.host,
            port: Some(credentials.port),
        })
    }
}

#[cfg(test)]
mod tests {
    //! Tests for Credentials: Display/FromStr parsing, serde round-trips, try_to_destination
    //! conversion, and find/find_strict/find_lax string scanning.

    use std::net::{Ipv4Addr, Ipv6Addr};

    use once_cell::sync::Lazy;
    use test_log::test;

    use super::*;

    const HOST: &str = "testhost";
    const PORT: u16 = 12345;

    const USER: &str = "testuser";
    static KEY: Lazy<String> = Lazy::new(|| SecretKey32::default().to_string());

    static CREDENTIALS_STR_NO_USER: Lazy<String> = Lazy::new(|| {
        let key = KEY.as_str();
        format!("distant://:{key}@{HOST}:{PORT}")
    });
    static CREDENTIALS_STR_USER: Lazy<String> = Lazy::new(|| {
        let key = KEY.as_str();
        format!("distant://{USER}:{key}@{HOST}:{PORT}")
    });

    static CREDENTIALS_NO_USER: Lazy<Credentials> =
        Lazy::new(|| CREDENTIALS_STR_NO_USER.parse().unwrap());
    static CREDENTIALS_USER: Lazy<Credentials> =
        Lazy::new(|| CREDENTIALS_STR_USER.parse().unwrap());

    #[test]
    fn find_should_return_some_key_if_string_is_exact_match() {
        let credentials = Credentials::find(CREDENTIALS_STR_NO_USER.as_str(), true);
        assert_eq!(credentials.unwrap(), *CREDENTIALS_NO_USER);

        let credentials = Credentials::find(CREDENTIALS_STR_USER.as_str(), true);
        assert_eq!(credentials.unwrap(), *CREDENTIALS_USER);
    }

    #[test]
    fn find_should_return_some_key_if_there_is_a_match_with_only_whitespace_on_either_side() {
        let s = format!(" {} ", CREDENTIALS_STR_NO_USER.as_str());
        let credentials = Credentials::find(&s, true);
        assert_eq!(credentials.unwrap(), *CREDENTIALS_NO_USER);

        let s = format!("\r{}\r", CREDENTIALS_STR_NO_USER.as_str());
        let credentials = Credentials::find(&s, true);
        assert_eq!(credentials.unwrap(), *CREDENTIALS_NO_USER);

        let s = format!("\t{}\t", CREDENTIALS_STR_NO_USER.as_str());
        let credentials = Credentials::find(&s, true);
        assert_eq!(credentials.unwrap(), *CREDENTIALS_NO_USER);

        let s = format!("\n{}\n", CREDENTIALS_STR_NO_USER.as_str());
        let credentials = Credentials::find(&s, true);
        assert_eq!(credentials.unwrap(), *CREDENTIALS_NO_USER);
    }

    #[test]
    fn find_should_return_some_key_if_there_is_a_match_with_only_control_characters_on_either_side()
    {
        let s = format!("\x1b{} \x1b", CREDENTIALS_STR_NO_USER.as_str());
        let credentials = Credentials::find(&s, true);
        assert_eq!(credentials.unwrap(), *CREDENTIALS_NO_USER);
    }

    #[test]
    fn find_should_return_first_match_found_in_str() {
        let s = format!(
            "{} {}",
            CREDENTIALS_STR_NO_USER.as_str(),
            CREDENTIALS_STR_USER.as_str()
        );
        let credentials = Credentials::find(&s, true);
        assert_eq!(credentials.unwrap(), *CREDENTIALS_NO_USER);
    }

    #[test]
    fn find_should_return_first_valid_match_found_in_str() {
        let s = format!(
            "a{}a {} b{}b",
            CREDENTIALS_STR_NO_USER.as_str(),
            CREDENTIALS_STR_NO_USER.as_str(),
            CREDENTIALS_STR_NO_USER.as_str()
        );
        let credentials = Credentials::find(&s, true);
        assert_eq!(credentials.unwrap(), *CREDENTIALS_NO_USER);
    }

    #[test]
    fn find_with_strict_false_should_ignore_any_character_preceding_scheme() {
        let s = format!("a{}", CREDENTIALS_STR_NO_USER.as_str());
        let credentials = Credentials::find(&s, false);
        assert_eq!(credentials.unwrap(), *CREDENTIALS_NO_USER);

        let s = format!(
            "a{} b{}",
            CREDENTIALS_STR_NO_USER.as_str(),
            CREDENTIALS_STR_NO_USER.as_str()
        );
        let credentials = Credentials::find(&s, false);
        assert_eq!(credentials.unwrap(), *CREDENTIALS_NO_USER);
    }

    #[test]
    fn find_with_strict_true_should_not_find_if_non_whitespace_and_control_preceding_scheme() {
        let s = format!("a{}", CREDENTIALS_STR_NO_USER.as_str());
        let credentials = Credentials::find(&s, true);
        assert_eq!(credentials, None);

        let s = format!(
            "a{} b{}",
            CREDENTIALS_STR_NO_USER.as_str(),
            CREDENTIALS_STR_NO_USER.as_str()
        );
        let credentials = Credentials::find(&s, true);
        assert_eq!(credentials, None);
    }

    #[test]
    fn find_should_return_none_if_no_match_found() {
        let s = "abc";
        let credentials = Credentials::find(s, true);
        assert_eq!(credentials, None);

        let s = "abc";
        let credentials = Credentials::find(s, false);
        assert_eq!(credentials, None);
    }

    #[test]
    fn display_should_not_wrap_ipv4_address() {
        let key = KEY.as_str();
        let credentials = Credentials {
            host: Host::Ipv4(Ipv4Addr::LOCALHOST),
            port: 12345,
            username: None,
            key: key.parse().unwrap(),
        };

        assert_eq!(
            credentials.to_string(),
            format!("{SCHEME}://:{key}@127.0.0.1:12345")
        );
    }

    #[test]
    fn display_should_wrap_ipv6_address_in_square_brackets() {
        let key = KEY.as_str();
        let credentials = Credentials {
            host: Host::Ipv6(Ipv6Addr::LOCALHOST),
            port: 12345,
            username: None,
            key: key.parse().unwrap(),
        };

        assert_eq!(
            credentials.to_string(),
            format!("{SCHEME}://:{key}@[::1]:12345")
        );
    }

    #[test]
    fn display_should_include_username_when_present() {
        let key = KEY.as_str();
        let credentials = Credentials {
            host: Host::Name(HOST.to_string()),
            port: PORT,
            username: Some(USER.to_string()),
            key: key.parse().unwrap(),
        };

        assert_eq!(
            credentials.to_string(),
            format!("{SCHEME}://{USER}:{key}@{HOST}:{PORT}")
        );
    }

    #[test]
    fn display_should_include_username_with_ipv6() {
        let key = KEY.as_str();
        let credentials = Credentials {
            host: Host::Ipv6(Ipv6Addr::LOCALHOST),
            port: PORT,
            username: Some(USER.to_string()),
            key: key.parse().unwrap(),
        };

        assert_eq!(
            credentials.to_string(),
            format!("{SCHEME}://{USER}:{key}@[::1]:{PORT}")
        );
    }

    #[test]
    fn display_should_handle_ipv4_with_username() {
        let key = KEY.as_str();
        let credentials = Credentials {
            host: Host::Ipv4(Ipv4Addr::new(192, 168, 1, 1)),
            port: 8080,
            username: Some("admin".to_string()),
            key: key.parse().unwrap(),
        };

        assert_eq!(
            credentials.to_string(),
            format!("{SCHEME}://admin:{key}@192.168.1.1:8080")
        );
    }

    #[test]
    fn from_str_should_parse_valid_credentials_without_username() {
        let credentials: Credentials = CREDENTIALS_STR_NO_USER.parse().unwrap();
        assert_eq!(credentials.host, Host::Name(HOST.to_string()));
        assert_eq!(credentials.port, PORT);
        assert!(credentials.username.is_none());
    }

    #[test]
    fn from_str_should_parse_valid_credentials_with_username() {
        let credentials: Credentials = CREDENTIALS_STR_USER.parse().unwrap();
        assert_eq!(credentials.host, Host::Name(HOST.to_string()));
        assert_eq!(credentials.port, PORT);
        assert_eq!(credentials.username.as_deref(), Some(USER));
    }

    #[test]
    fn from_str_should_fail_with_wrong_scheme() {
        let key = KEY.as_str();
        let s = format!("ssh://:{key}@{HOST}:{PORT}");
        let result: Result<Credentials, _> = s.parse();
        assert!(result.is_err());
    }

    #[test]
    fn from_str_should_fail_when_port_is_missing() {
        let key = KEY.as_str();
        let s = format!("distant://:{key}@{HOST}");
        let result: Result<Credentials, _> = s.parse();
        assert!(result.is_err());
    }

    #[test]
    fn from_str_should_fail_when_key_is_missing() {
        let s = format!("distant://@{HOST}:{PORT}");
        let result: Result<Credentials, _> = s.parse();
        assert!(result.is_err());
    }

    #[test]
    fn from_str_should_fail_with_invalid_key() {
        let s = format!("distant://:not-a-valid-hex-key@{HOST}:{PORT}");
        let result: Result<Credentials, _> = s.parse();
        assert!(result.is_err());
    }

    #[test]
    fn from_str_should_parse_ipv4_host() {
        let key = KEY.as_str();
        let s = format!("distant://:{key}@127.0.0.1:{PORT}");
        let credentials: Credentials = s.parse().unwrap();
        assert_eq!(credentials.host, Host::Ipv4(Ipv4Addr::LOCALHOST));
        assert_eq!(credentials.port, PORT);
    }

    #[test]
    fn from_str_should_parse_ipv6_host() {
        let key = KEY.as_str();
        let s = format!("distant://:{key}@[::1]:{PORT}");
        let credentials: Credentials = s.parse().unwrap();
        assert_eq!(credentials.host, Host::Ipv6(Ipv6Addr::LOCALHOST));
        assert_eq!(credentials.port, PORT);
    }

    #[test]
    fn round_trip_display_and_parse_should_preserve_credentials_with_hostname() {
        let credentials = &*CREDENTIALS_NO_USER;
        let s = credentials.to_string();
        let parsed: Credentials = s.parse().unwrap();
        assert_eq!(&parsed, credentials);
    }

    #[test]
    fn round_trip_display_and_parse_should_preserve_credentials_with_username() {
        let credentials = &*CREDENTIALS_USER;
        let s = credentials.to_string();
        let parsed: Credentials = s.parse().unwrap();
        assert_eq!(&parsed, credentials);
    }

    #[test]
    fn round_trip_display_and_parse_should_preserve_ipv4_credentials() {
        let key = KEY.as_str();
        let credentials = Credentials {
            host: Host::Ipv4(Ipv4Addr::LOCALHOST),
            port: PORT,
            username: None,
            key: key.parse().unwrap(),
        };
        let s = credentials.to_string();
        let parsed: Credentials = s.parse().unwrap();
        assert_eq!(parsed, credentials);
    }

    #[test]
    fn round_trip_display_and_parse_should_preserve_ipv6_credentials() {
        let key = KEY.as_str();
        let credentials = Credentials {
            host: Host::Ipv6(Ipv6Addr::LOCALHOST),
            port: PORT,
            username: None,
            key: key.parse().unwrap(),
        };
        let s = credentials.to_string();
        let parsed: Credentials = s.parse().unwrap();
        assert_eq!(parsed, credentials);
    }

    #[test]
    fn round_trip_display_and_parse_should_preserve_ipv6_credentials_with_username() {
        let key = KEY.as_str();
        let credentials = Credentials {
            host: Host::Ipv6(Ipv6Addr::LOCALHOST),
            port: PORT,
            username: Some(USER.to_string()),
            key: key.parse().unwrap(),
        };
        let s = credentials.to_string();
        let parsed: Credentials = s.parse().unwrap();
        assert_eq!(parsed, credentials);
    }

    #[test]
    fn serde_json_round_trip_should_preserve_credentials() {
        let credentials = &*CREDENTIALS_NO_USER;
        let json = serde_json::to_string(credentials).unwrap();
        let deserialized: Credentials = serde_json::from_str(&json).unwrap();
        assert_eq!(&deserialized, credentials);
    }

    #[test]
    fn serde_json_round_trip_should_preserve_credentials_with_username() {
        let credentials = &*CREDENTIALS_USER;
        let json = serde_json::to_string(credentials).unwrap();
        let deserialized: Credentials = serde_json::from_str(&json).unwrap();
        assert_eq!(&deserialized, credentials);
    }

    #[test]
    fn try_to_destination_should_produce_valid_destination() {
        let credentials = &*CREDENTIALS_USER;
        let dest = credentials.try_to_destination().unwrap();

        assert_eq!(dest.scheme.as_deref(), Some("distant"));
        assert_eq!(dest.username.as_deref(), Some(USER));
        assert!(dest.password.is_some());
        assert_eq!(dest.host, Host::Name(HOST.to_string()));
        assert_eq!(dest.port, Some(PORT));
    }

    #[test]
    fn try_to_destination_should_produce_valid_destination_without_username() {
        let credentials = &*CREDENTIALS_NO_USER;
        let dest = credentials.try_to_destination().unwrap();

        assert_eq!(dest.scheme.as_deref(), Some("distant"));
        assert!(dest.username.is_none());
        assert!(dest.password.is_some());
        assert_eq!(dest.host, Host::Name(HOST.to_string()));
        assert_eq!(dest.port, Some(PORT));
    }

    #[test]
    fn try_from_credentials_for_destination_should_match_try_to_destination() {
        let credentials = &*CREDENTIALS_USER;
        let dest1 = credentials.try_to_destination().unwrap();
        let dest2: Destination = Destination::try_from(credentials.clone()).unwrap();
        assert_eq!(dest1, dest2);
    }

    #[test]
    fn find_strict_should_be_equivalent_to_find_with_strict_true() {
        let result_strict = Credentials::find_strict(CREDENTIALS_STR_NO_USER.as_str());
        let result_find = Credentials::find(CREDENTIALS_STR_NO_USER.as_str(), true);
        assert_eq!(result_strict, result_find);
    }

    #[test]
    fn find_lax_should_be_equivalent_to_find_with_strict_false() {
        let result_lax = Credentials::find_lax(CREDENTIALS_STR_NO_USER.as_str());
        let result_find = Credentials::find(CREDENTIALS_STR_NO_USER.as_str(), false);
        assert_eq!(result_lax, result_find);
    }

    #[test]
    fn find_strict_should_reject_non_boundary_prefix() {
        let s = format!("x{}", CREDENTIALS_STR_NO_USER.as_str());
        assert!(Credentials::find_strict(&s).is_none());
    }

    #[test]
    fn find_lax_should_accept_non_boundary_prefix() {
        let s = format!("x{}", CREDENTIALS_STR_NO_USER.as_str());
        let result = Credentials::find_lax(&s);
        assert_eq!(result.unwrap(), *CREDENTIALS_NO_USER);
    }

    #[test]
    fn find_should_return_none_for_empty_string() {
        assert!(Credentials::find("", true).is_none());
        assert!(Credentials::find("", false).is_none());
    }

    #[test]
    fn find_should_handle_trailing_content_after_credentials() {
        let s = format!("{}\n\nsome trailing text", CREDENTIALS_STR_NO_USER.as_str());
        let result = Credentials::find(&s, true);
        assert_eq!(result.unwrap(), *CREDENTIALS_NO_USER);
    }

    #[test]
    fn clone_should_produce_equal_credentials() {
        let credentials = &*CREDENTIALS_USER;
        let cloned = credentials.clone();
        assert_eq!(&cloned, credentials);
    }
}
