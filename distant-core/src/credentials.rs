use super::serde::{deserialize_from_str, serialize_to_str};
use crate::Destination;
use distant_net::SecretKey32;
use serde::{de::Deserializer, ser::Serializer, Deserialize, Serialize};
use std::{
    convert::{TryFrom, TryInto},
    fmt, io,
    str::FromStr,
};
use uriparse::{URIReference, URI};

/// Represents credentials used for a distant server that is maintaining a single key
/// across all connections
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DistantSingleKeyCredentials {
    pub host: String,
    pub port: u16,
    pub key: SecretKey32,
    pub username: Option<String>,
}

impl fmt::Display for DistantSingleKeyCredentials {
    /// Converts credentials into string in the form of `distant://[username]:{key}@{host}:{port}`
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "distant://")?;
        if let Some(username) = self.username.as_ref() {
            write!(f, "{}", username)?;
        }
        write!(f, ":{}@{}:{}", self.key, self.host, self.port)
    }
}

impl FromStr for DistantSingleKeyCredentials {
    type Err = io::Error;

    /// Parse `distant://[username]:{key}@{host}` as credentials. Note that this requires the
    /// `distant` scheme to be included. If parsing without scheme is desired, call the
    /// [`DistantSingleKeyCredentials::try_from_uri_ref`] method instead with `require_scheme`
    /// set to false
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from_uri_ref(s, true)
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
    /// Converts credentials into a [`Destination`] of the form `distant://[username]:{key}@{host}`,
    /// failing if the credentials would not produce a valid [`Destination`]
    pub fn try_to_destination(&self) -> io::Result<Destination> {
        let uri = self.try_to_uri()?;
        Destination::try_from(uri.as_uri_reference().to_borrowed())
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))
    }

    /// Converts credentials into a [`URI`] of the form `distant://[username]:{key}@{host}`,
    /// failing if the credentials would not produce a valid [`URI`]
    pub fn try_to_uri(&self) -> io::Result<URI<'static>> {
        let uri_string = self.to_string();
        URI::try_from(uri_string.as_str())
            .map(URI::into_owned)
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))
    }

    /// Parses credentials from a [`URIReference`], failing if the input was not a valid
    /// [`URIReference`] or if required parameters like `host` or `password` are missing or bad
    /// format
    ///
    /// If `require_scheme` is true, will enforce that a scheme is provided. Regardless, if a
    /// scheme is provided that is not `distant`, this will also fail
    pub fn try_from_uri_ref<'a, E>(
        uri_ref: impl TryInto<URIReference<'a>, Error = E>,
        require_scheme: bool,
    ) -> io::Result<Self>
    where
        E: std::error::Error + Send + Sync + 'static,
    {
        let uri_ref = uri_ref
            .try_into()
            .map_err(|x| io::Error::new(io::ErrorKind::Other, x))?;

        // Check if the scheme is correct, and if missing if we require it
        if let Some(scheme) = uri_ref.scheme() {
            if !scheme.as_str().eq_ignore_ascii_case("distant") {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Scheme is not distant",
                ));
            }
        } else if require_scheme {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Missing scheme",
            ));
        }

        Ok(Self {
            host: uri_ref
                .host()
                .map(ToString::to_string)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Missing host"))?,
            port: uri_ref
                .port()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Missing port"))?,
            key: uri_ref
                .password()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Missing password"))
                .and_then(|x| {
                    x.parse()
                        .map_err(|x| io::Error::new(io::ErrorKind::InvalidInput, x))
                })?,
            username: uri_ref.username().map(ToString::to_string),
        })
    }
}
