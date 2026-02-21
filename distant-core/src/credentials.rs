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
pub struct DistantSingleKeyCredentials {
    pub host: Host,
    pub port: u16,
    pub key: SecretKey32,
    pub username: Option<String>,
}

impl fmt::Display for DistantSingleKeyCredentials {
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

impl FromStr for DistantSingleKeyCredentials {
    type Err = io::Error;

    /// Parse `distant://[username]:{key}@{host}:{port}` as credentials. Note that this requires the
    /// `distant` scheme to be included. If parsing without scheme is desired, call the
    /// [`DistantSingleKeyCredentials::try_from_uri_ref`] method instead with `require_scheme`
    /// set to false
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let destination: Destination = s
            .parse()
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))?;

        // Can be scheme-less or explicitly distant
        if let Some(scheme) = destination.scheme.as_deref() {
            if scheme != SCHEME {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Unexpected scheme: {scheme}"),
                ));
            }
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

impl Serialize for DistantSingleKeyCredentials {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serialize_to_str(self, serializer)
    }
}

impl<'de> Deserialize<'de> for DistantSingleKeyCredentials {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_from_str(deserializer)
    }
}

impl DistantSingleKeyCredentials {
    /// Searches a str for `distant://[username]:{key}@{host}:{port}`, returning the first matching
    /// credentials set if found, failing if anything is found immediately before or after the
    /// credentials that is not whitespace or control characters
    ///
    /// If `strict` is false, then the scheme can be preceded by any character
    pub fn find(s: &str, strict: bool) -> Option<DistantSingleKeyCredentials> {
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
    /// [`find(s, true)`]: DistantSingleKeyCredentials::find
    #[inline]
    pub fn find_strict(s: &str) -> Option<DistantSingleKeyCredentials> {
        Self::find(s, true)
    }

    /// Equivalent to [`find(s, false)`].
    ///
    /// [`find(s, false)`]: DistantSingleKeyCredentials::find
    #[inline]
    pub fn find_lax(s: &str) -> Option<DistantSingleKeyCredentials> {
        Self::find(s, false)
    }

    /// Converts credentials into a [`Destination`] of the form
    /// `distant://[username]:{key}@{host}:{port}`, failing if the credentials would not produce a
    /// valid [`Destination`]
    pub fn try_to_destination(&self) -> io::Result<Destination> {
        TryFrom::try_from(self.clone())
    }
}

impl TryFrom<DistantSingleKeyCredentials> for Destination {
    type Error = io::Error;

    fn try_from(credentials: DistantSingleKeyCredentials) -> Result<Self, Self::Error> {
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

    static CREDENTIALS_NO_USER: Lazy<DistantSingleKeyCredentials> =
        Lazy::new(|| CREDENTIALS_STR_NO_USER.parse().unwrap());
    static CREDENTIALS_USER: Lazy<DistantSingleKeyCredentials> =
        Lazy::new(|| CREDENTIALS_STR_USER.parse().unwrap());

    #[test]
    fn find_should_return_some_key_if_string_is_exact_match() {
        let credentials = DistantSingleKeyCredentials::find(CREDENTIALS_STR_NO_USER.as_str(), true);
        assert_eq!(credentials.unwrap(), *CREDENTIALS_NO_USER);

        let credentials = DistantSingleKeyCredentials::find(CREDENTIALS_STR_USER.as_str(), true);
        assert_eq!(credentials.unwrap(), *CREDENTIALS_USER);
    }

    #[test]
    fn find_should_return_some_key_if_there_is_a_match_with_only_whitespace_on_either_side() {
        let s = format!(" {} ", CREDENTIALS_STR_NO_USER.as_str());
        let credentials = DistantSingleKeyCredentials::find(&s, true);
        assert_eq!(credentials.unwrap(), *CREDENTIALS_NO_USER);

        let s = format!("\r{}\r", CREDENTIALS_STR_NO_USER.as_str());
        let credentials = DistantSingleKeyCredentials::find(&s, true);
        assert_eq!(credentials.unwrap(), *CREDENTIALS_NO_USER);

        let s = format!("\t{}\t", CREDENTIALS_STR_NO_USER.as_str());
        let credentials = DistantSingleKeyCredentials::find(&s, true);
        assert_eq!(credentials.unwrap(), *CREDENTIALS_NO_USER);

        let s = format!("\n{}\n", CREDENTIALS_STR_NO_USER.as_str());
        let credentials = DistantSingleKeyCredentials::find(&s, true);
        assert_eq!(credentials.unwrap(), *CREDENTIALS_NO_USER);
    }

    #[test]
    fn find_should_return_some_key_if_there_is_a_match_with_only_control_characters_on_either_side()
    {
        let s = format!("\x1b{} \x1b", CREDENTIALS_STR_NO_USER.as_str());
        let credentials = DistantSingleKeyCredentials::find(&s, true);
        assert_eq!(credentials.unwrap(), *CREDENTIALS_NO_USER);
    }

    #[test]
    fn find_should_return_first_match_found_in_str() {
        let s = format!(
            "{} {}",
            CREDENTIALS_STR_NO_USER.as_str(),
            CREDENTIALS_STR_USER.as_str()
        );
        let credentials = DistantSingleKeyCredentials::find(&s, true);
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
        let credentials = DistantSingleKeyCredentials::find(&s, true);
        assert_eq!(credentials.unwrap(), *CREDENTIALS_NO_USER);
    }

    #[test]
    fn find_with_strict_false_should_ignore_any_character_preceding_scheme() {
        let s = format!("a{}", CREDENTIALS_STR_NO_USER.as_str());
        let credentials = DistantSingleKeyCredentials::find(&s, false);
        assert_eq!(credentials.unwrap(), *CREDENTIALS_NO_USER);

        let s = format!(
            "a{} b{}",
            CREDENTIALS_STR_NO_USER.as_str(),
            CREDENTIALS_STR_NO_USER.as_str()
        );
        let credentials = DistantSingleKeyCredentials::find(&s, false);
        assert_eq!(credentials.unwrap(), *CREDENTIALS_NO_USER);
    }

    #[test]
    fn find_with_strict_true_should_not_find_if_non_whitespace_and_control_preceding_scheme() {
        let s = format!("a{}", CREDENTIALS_STR_NO_USER.as_str());
        let credentials = DistantSingleKeyCredentials::find(&s, true);
        assert_eq!(credentials, None);

        let s = format!(
            "a{} b{}",
            CREDENTIALS_STR_NO_USER.as_str(),
            CREDENTIALS_STR_NO_USER.as_str()
        );
        let credentials = DistantSingleKeyCredentials::find(&s, true);
        assert_eq!(credentials, None);
    }

    #[test]
    fn find_should_return_none_if_no_match_found() {
        let s = "abc";
        let credentials = DistantSingleKeyCredentials::find(s, true);
        assert_eq!(credentials, None);

        let s = "abc";
        let credentials = DistantSingleKeyCredentials::find(s, false);
        assert_eq!(credentials, None);
    }

    #[test]
    fn display_should_not_wrap_ipv4_address() {
        let key = KEY.as_str();
        let credentials = DistantSingleKeyCredentials {
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
        let credentials = DistantSingleKeyCredentials {
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
}
