use super::serde::{deserialize_from_str, serialize_to_str};
use derive_more::{Display, Error, From};
use serde::{de::Deserializer, ser::Serializer, Deserialize, Serialize};
use std::{convert::TryFrom, fmt, hash::Hash, str::FromStr};
use uriparse::{
    Authority, AuthorityError, Host, Password, Scheme, URIReference, URIReferenceError, Username,
    URI,
};

/// Represents an error that occurs when trying to parse a destination from a str
#[derive(Copy, Clone, Debug, Display, Error, From, PartialEq, Eq)]
pub enum DestinationError {
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
pub struct Destination(URIReference<'static>);

impl Destination {
    /// Returns a reference to the scheme associated with the destination, if it has one
    pub fn scheme(&self) -> Option<&str> {
        self.0.scheme().map(Scheme::as_str)
    }

    /// Returns the host of the destination as a string
    pub fn to_host_string(&self) -> String {
        // NOTE: We guarantee that there is a host for a destination during construction
        self.0.host().unwrap().to_string()
    }

    /// Returns the port tied to the destination, if it has one
    pub fn port(&self) -> Option<u16> {
        self.0.port()
    }

    /// Returns the username tied with the destination if it has one
    pub fn username(&self) -> Option<&str> {
        self.0.username().map(Username::as_str)
    }

    /// Returns the password tied with the destination if it has one
    pub fn password(&self) -> Option<&str> {
        self.0.password().map(Password::as_str)
    }

    /// Replaces the host of the destination
    pub fn replace_host(&mut self, host: &str) -> Result<(), URIReferenceError> {
        let username = self
            .0
            .username()
            .map(Username::as_borrowed)
            .map(Username::into_owned);
        let password = self
            .0
            .password()
            .map(Password::as_borrowed)
            .map(Password::into_owned);
        let port = self.0.port();
        let _ = self.0.set_authority(Some(
            Authority::from_parts(
                username,
                password,
                Host::try_from(host)
                    .map(Host::into_owned)
                    .map_err(AuthorityError::from)
                    .map_err(URIReferenceError::from)?,
                port,
            )
            .map(Authority::into_owned)
            .map_err(URIReferenceError::from)?,
        ))?;
        Ok(())
    }

    /// Indicates whether the host destination is globally routable
    pub fn is_host_global(&self) -> bool {
        match self.0.host() {
            Some(Host::IPv4Address(x)) => {
                !(x.is_broadcast()
                    || x.is_documentation()
                    || x.is_link_local()
                    || x.is_loopback()
                    || x.is_private()
                    || x.is_unspecified())
            }
            Some(Host::IPv6Address(x)) => {
                // NOTE: 14 is the global flag
                x.is_multicast() && (x.segments()[0] & 0x000f == 14)
            }
            Some(Host::RegisteredName(name)) => !name.trim().is_empty(),
            None => false,
        }
    }

    /// Returns true if destination represents a distant server
    pub fn is_distant(&self) -> bool {
        self.scheme_eq("distant")
    }

    /// Returns true if destination represents an ssh server
    pub fn is_ssh(&self) -> bool {
        self.scheme_eq("ssh")
    }

    fn scheme_eq(&self, s: &str) -> bool {
        match self.0.scheme() {
            Some(scheme) => scheme.as_str().eq_ignore_ascii_case(s),
            None => false,
        }
    }

    /// Returns reference to inner [`URIReference`]
    pub fn as_uri_ref(&self) -> &URIReference<'static> {
        &self.0
    }
}

impl AsRef<Destination> for &Destination {
    fn as_ref(&self) -> &Destination {
        *self
    }
}

impl AsRef<URIReference<'static>> for Destination {
    fn as_ref(&self) -> &URIReference<'static> {
        self.as_uri_ref()
    }
}

impl fmt::Display for Destination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Destination {
    type Err = DestinationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Disallow empty (whitespace-only) input as that passes our
        // parsing for a URI reference (relative with no scheme or anything)
        if s.trim().is_empty() {
            return Err(DestinationError::MissingHost);
        }

        let mut destination = URIReference::try_from(s)
            .map(URIReference::into_owned)
            .map(Destination)
            .map_err(DestinationError::URIReferenceError)?;

        // Only support relative reference if it is a path reference as
        // we convert that to a relative reference with a host
        if destination.0.is_relative_reference() {
            let path = destination.0.path().to_string();
            destination.replace_host(path.as_str())?;
            let _ = destination.0.set_path("/")?;
        }

        Ok(destination)
    }
}

impl<'a> TryFrom<URIReference<'a>> for Destination {
    type Error = DestinationError;

    fn try_from(uri_ref: URIReference<'a>) -> Result<Self, Self::Error> {
        if uri_ref.host().is_none() {
            return Err(DestinationError::MissingHost);
        }

        Ok(Self(uri_ref.into_owned()))
    }
}

impl<'a> TryFrom<URI<'a>> for Destination {
    type Error = DestinationError;

    fn try_from(uri: URI<'a>) -> Result<Self, Self::Error> {
        let uri_ref: URIReference<'a> = uri.into();
        Self::try_from(uri_ref)
    }
}

impl FromStr for Box<Destination> {
    type Err = DestinationError;

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
        assert_eq!(err, DestinationError::MissingHost);

        let err = " ".parse::<Destination>().unwrap_err();
        assert_eq!(err, DestinationError::MissingHost);

        let err = "\t".parse::<Destination>().unwrap_err();
        assert_eq!(err, DestinationError::MissingHost);

        let err = "\n".parse::<Destination>().unwrap_err();
        assert_eq!(err, DestinationError::MissingHost);

        let err = "\r".parse::<Destination>().unwrap_err();
        assert_eq!(err, DestinationError::MissingHost);

        let err = "\r\n".parse::<Destination>().unwrap_err();
        assert_eq!(err, DestinationError::MissingHost);
    }

    #[test]
    fn parse_should_succeed_with_valid_uri() {
        let destination = "distant://localhost".parse::<Destination>().unwrap();
        assert_eq!(destination.scheme().unwrap(), "distant");
        assert_eq!(destination.to_host_string(), "localhost");
        assert_eq!(destination.as_uri_ref().path().to_string(), "/");
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
        assert_eq!(destination.to_host_string(), "localhost");
        assert_eq!(destination.as_uri_ref().path().to_string(), "/");
    }
}
