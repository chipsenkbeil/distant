use derive_more::{Display, Error};
use distant_core::net::{SecretKey32, UnprotectedToHexKey};
use std::{
    env, fmt, io,
    net::{IpAddr, SocketAddr},
    str::FromStr,
};
use tokio::net::lookup_host;

/// Represents information after launching a server
#[derive(Debug, PartialEq, Eq)]
pub struct LaunchInfo {
    pub host: String,
    pub port: u16,
    pub key: SecretKey32,
}

#[derive(Copy, Clone, Debug, Display, Error, PartialEq, Eq)]
pub enum LaunchInfoParseError {
    #[display(fmt = "Prefix of string is invalid")]
    BadPrefix,

    #[display(fmt = "Bad hex key for launch info")]
    BadHexKey,

    #[display(fmt = "Invalid key for launch info")]
    InvalidKey,

    #[display(fmt = "Invalid port for launch info")]
    InvalidPort,

    #[display(fmt = "Missing address for launch info")]
    MissingAddr,

    #[display(fmt = "Missing key for launch info")]
    MissingKey,

    #[display(fmt = "Missing port for launch info")]
    MissingPort,
}

impl From<LaunchInfoParseError> for io::Error {
    fn from(x: LaunchInfoParseError) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, x)
    }
}

impl FromStr for LaunchInfo {
    type Err = LaunchInfoParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut tokens = s.trim().split(' ').take(5);

        // First, validate that we have the appropriate prefix
        if tokens.next().ok_or(LaunchInfoParseError::BadPrefix)? != "DISTANT" {
            return Err(LaunchInfoParseError::BadPrefix);
        }
        if tokens.next().ok_or(LaunchInfoParseError::BadPrefix)? != "CONNECT" {
            return Err(LaunchInfoParseError::BadPrefix);
        }

        // Second, load up the address without parsing it
        let host = tokens
            .next()
            .ok_or(LaunchInfoParseError::MissingAddr)?
            .trim()
            .to_string();

        // Third, load up the port and parse it into a number
        let port = tokens
            .next()
            .ok_or(LaunchInfoParseError::MissingPort)?
            .trim()
            .parse::<u16>()
            .map_err(|_| LaunchInfoParseError::InvalidPort)?;

        // Fourth, load up the key and convert it back into a secret key from a hex slice
        let key = SecretKey32::from_slice(
            &hex::decode(
                tokens
                    .next()
                    .ok_or(LaunchInfoParseError::MissingKey)?
                    .trim(),
            )
            .map_err(|_| LaunchInfoParseError::BadHexKey)?,
        )
        .map_err(|_| LaunchInfoParseError::InvalidKey)?;

        Ok(LaunchInfo { host, port, key })
    }
}

impl fmt::Display for LaunchInfo {
    /// Writes out `DISTANT CONNECT {host} {port} {key}`
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "DISTANT CONNECT {} {} {}",
            self.host,
            self.port,
            self.key.unprotected_to_hex_key()
        )
    }
}

impl LaunchInfo {
    /// Loads launch info from environment variables
    pub fn from_environment() -> io::Result<Self> {
        fn to_err(x: env::VarError) -> io::Error {
            io::Error::new(io::ErrorKind::InvalidInput, x)
        }

        let host = env::var("DISTANT_HOST").map_err(to_err)?;
        let port = env::var("DISTANT_PORT").map_err(to_err)?;
        let key = env::var("DISTANT_KEY").map_err(to_err)?;
        Ok(format!("DISTANT CONNECT {} {} {}", host, port, key).parse()?)
    }

    /// Loads launch info from the next line available in this program's stdin
    pub fn from_stdin() -> io::Result<Self> {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        line.parse()
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))
    }

    /// Consumes the launch info and returns the key
    pub fn into_key(self) -> SecretKey32 {
        self.key
    }

    /// Returns the ip address associated with the launch info based on the host
    pub async fn to_ip_addr(&self) -> io::Result<IpAddr> {
        let addr = match self.host.parse::<IpAddr>() {
            Ok(addr) => addr,
            Err(_) => lookup_host((self.host.as_str(), self.port))
                .await?
                .next()
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "Failed to lookup_host"))?
                .ip(),
        };

        Ok(addr)
    }

    /// Returns socket address associated with the launch info
    pub async fn to_socket_addr(&self) -> io::Result<SocketAddr> {
        let addr = self.to_ip_addr().await?;
        Ok(SocketAddr::from((addr, self.port)))
    }

    /// Converts the launch info's key to a hex string
    pub fn key_to_unprotected_string(&self) -> String {
        self.key.unprotected_to_hex_key()
    }
}
