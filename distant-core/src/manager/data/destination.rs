use super::serde::{deserialize_from_str, serialize_to_str};
use derive_more::{Display, Error, From};
use serde::{de::Deserializer, ser::Serializer, Deserialize, Serialize};
use std::{
    convert::TryFrom,
    fmt,
    hash::Hash,
    ops::{Deref, DerefMut},
    str::FromStr,
};
use uriparse::{
    Authority, AuthorityError, Host, Password, URIReference, URIReferenceError, Username,
};

/// Represents an error that occurs when trying to parse a destination from a str
#[derive(Copy, Clone, Debug, Display, Error, From, PartialEq, Eq)]
pub enum DestinationParseError {
    MissingHost,
    URIReferenceError(URIReferenceError),
}

/// `distant` connects and logs into the specified destination, which may be specified as either
/// `hostname:port` where an attempt to connect to a **distant** server will be made, or a URI of
/// one of the following forms:
///
/// * `distant://hostname:port` - connect to a distant server
/// * `ssh://[user@]hostname[:port]` - connect to an SSH server
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct Destination(pub(crate) URIReference<'static>);

impl Destination {
    /// Returns true if destination represents a distant server
    pub fn is_distant(&self) -> bool {
        match self.scheme() {
            Some(scheme) => scheme.as_str().eq_ignore_ascii_case("distant"),

            // Without scheme, distant is usd by default
            None => true,
        }
    }

    /// Returns true if destination represents an ssh server
    pub fn is_ssh(&self) -> bool {
        match self.scheme() {
            Some(scheme) => scheme.as_str().eq_ignore_ascii_case("ssh"),
            None => false,
        }
    }
}

impl AsRef<Destination> for &Destination {
    fn as_ref(&self) -> &Destination {
        *self
    }
}

impl Deref for Destination {
    type Target = URIReference<'static>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for Destination {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl fmt::Display for Destination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Destination {
    type Err = DestinationParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Disallow empty (whitespace-only) input as that passes our
        // parsing for a URI reference (relative with no scheme or anything)
        if s.trim().is_empty() {
            return Err(DestinationParseError::MissingHost);
        }

        let mut destination = URIReference::try_from(s)
            .map(URIReference::into_owned)
            .map(Destination)
            .map_err(DestinationParseError::URIReferenceError)?;

        // Only support relative reference if it is a path reference as
        // we convert that to a relative reference with a host
        if destination.is_relative_reference() {
            let path = destination.path().to_string();
            let _ = destination.set_authority(Some(
                Authority::from_parts(
                    None::<Username>,
                    None::<Password>,
                    Host::try_from(path.as_str())
                        .map(Host::into_owned)
                        .map_err(AuthorityError::from)
                        .map_err(URIReferenceError::from)?,
                    None,
                )
                .map_err(URIReferenceError::from)?,
            ))?;
            let _ = destination.set_path("/")?;
        }

        Ok(destination)
    }
}

impl FromStr for Box<Destination> {
    type Err = DestinationParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let destination = s.parse::<Destination>()?;
        Ok(Box::new(destination))
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
    fn parse_should_fail_if_string_is_only_whitespace() {
        let err = "".parse::<Destination>().unwrap_err();
        assert_eq!(err, DestinationParseError::MissingHost);

        let err = " ".parse::<Destination>().unwrap_err();
        assert_eq!(err, DestinationParseError::MissingHost);

        let err = "\t".parse::<Destination>().unwrap_err();
        assert_eq!(err, DestinationParseError::MissingHost);

        let err = "\n".parse::<Destination>().unwrap_err();
        assert_eq!(err, DestinationParseError::MissingHost);

        let err = "\r".parse::<Destination>().unwrap_err();
        assert_eq!(err, DestinationParseError::MissingHost);

        let err = "\r\n".parse::<Destination>().unwrap_err();
        assert_eq!(err, DestinationParseError::MissingHost);
    }

    #[test]
    fn parse_should_succeed_with_valid_uri() {
        let destination = "distant://localhost".parse::<Destination>().unwrap();
        assert_eq!(destination.scheme().unwrap(), "distant");
        assert_eq!(destination.host().unwrap().to_string(), "localhost");
        assert_eq!(destination.path().to_string(), "/");
    }

    #[test]
    fn parse_should_fail_if_relative_reference_that_is_not_valid_host() {
        let _ = "/".parse::<Destination>().unwrap_err();
        let _ = "/localhost".parse::<Destination>().unwrap_err();
        let _ = "my/path".parse::<Destination>().unwrap_err();
        let _ = "/my/path".parse::<Destination>().unwrap_err();
        let _ = "//localhost".parse::<Destination>().unwrap_err();
    }

    #[test]
    fn parse_should_succeed_with_nonempty_relative_reference_by_setting_host_to_path() {
        let destination = "localhost".parse::<Destination>().unwrap();
        assert_eq!(destination.host().unwrap().to_string(), "localhost");
        assert_eq!(destination.path().to_string(), "/");
    }
}
