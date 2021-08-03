use crate::{PROJECT_DIRS, SESSION_PATH};
use derive_more::{Display, Error};
use orion::aead::SecretKey;
use std::{
    env,
    net::{IpAddr, SocketAddr},
    ops::Deref,
    path::Path,
    str::FromStr,
};
use tokio::{io, net::lookup_host};

#[derive(Debug, PartialEq, Eq)]
pub struct Session {
    pub host: String,
    pub port: u16,
    pub auth_key: SecretKey,
}

#[derive(Copy, Clone, Debug, Display, Error, PartialEq, Eq)]
pub enum SessionParseError {
    #[display(fmt = "Prefix of string is invalid")]
    BadPrefix,

    #[display(fmt = "Bad hex key for session")]
    BadSessionHexKey,

    #[display(fmt = "Invalid key for session")]
    InvalidSessionKey,

    #[display(fmt = "Invalid port for session")]
    InvalidSessionPort,

    #[display(fmt = "Missing address for session")]
    MissingSessionAddr,

    #[display(fmt = "Missing key for session")]
    MissingSessionKey,

    #[display(fmt = "Missing port for session")]
    MissingSessionPort,
}

impl From<SessionParseError> for io::Error {
    fn from(x: SessionParseError) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, x)
    }
}

impl FromStr for Session {
    type Err = SessionParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut tokens = s.split(' ').take(5);

        // First, validate that we have the appropriate prefix
        if tokens.next().ok_or(SessionParseError::BadPrefix)? != "DISTANT" {
            return Err(SessionParseError::BadPrefix);
        }
        if tokens.next().ok_or(SessionParseError::BadPrefix)? != "DATA" {
            return Err(SessionParseError::BadPrefix);
        }

        // Second, load up the address without parsing it
        let host = tokens
            .next()
            .ok_or(SessionParseError::MissingSessionAddr)?
            .trim()
            .to_string();

        // Third, load up the port and parse it into a number
        let port = tokens
            .next()
            .ok_or(SessionParseError::MissingSessionPort)?
            .trim()
            .parse::<u16>()
            .map_err(|_| SessionParseError::InvalidSessionPort)?;

        // Fourth, load up the key and convert it back into a secret key from a hex slice
        let auth_key = SecretKey::from_slice(
            &hex::decode(
                tokens
                    .next()
                    .ok_or(SessionParseError::MissingSessionKey)?
                    .trim(),
            )
            .map_err(|_| SessionParseError::BadSessionHexKey)?,
        )
        .map_err(|_| SessionParseError::InvalidSessionKey)?;

        Ok(Session {
            host,
            port,
            auth_key,
        })
    }
}

impl Session {
    /// Loads session from environment variables
    pub fn from_environment() -> io::Result<Self> {
        fn to_err(x: env::VarError) -> io::Error {
            io::Error::new(io::ErrorKind::InvalidInput, x)
        }

        let host = env::var("DISTANT_HOST").map_err(to_err)?;
        let port = env::var("DISTANT_PORT").map_err(to_err)?;
        let auth_key = env::var("DISTANT_AUTH_KEY").map_err(to_err)?;
        Ok(format!("DISTANT DATA {} {} {}", host, port, auth_key).parse()?)
    }

    /// Loads session from the next line available in this program's stdin
    pub fn from_stdin() -> io::Result<Self> {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line)?;
        line.parse()
            .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, x))
    }

    /// Returns the ip address associated with the session based on the host
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

    /// Returns socket address associated with the session
    pub async fn to_socket_addr(&self) -> io::Result<SocketAddr> {
        let addr = self.to_ip_addr().await?;
        Ok(SocketAddr::from((addr, self.port)))
    }

    /// Returns a string representing the auth key as hex
    pub fn to_unprotected_hex_auth_key(&self) -> String {
        hex::encode(self.auth_key.unprotected_as_bytes())
    }

    /// Converts to unprotected string that exposes the auth key in the form of
    /// `DISTANT DATA <addr> <port> <auth key>`
    pub async fn to_unprotected_string(&self) -> io::Result<String> {
        Ok(format!(
            "DISTANT DATA {} {} {}",
            self.to_ip_addr().await?,
            self.port,
            self.to_unprotected_hex_auth_key()
        ))
    }
}

/// Provides operations related to working with a session that is disk-based
pub struct SessionFile(Session);

impl AsRef<Session> for SessionFile {
    fn as_ref(&self) -> &Session {
        &self.0
    }
}

impl Deref for SessionFile {
    type Target = Session;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<SessionFile> for Session {
    fn from(sf: SessionFile) -> Self {
        sf.0
    }
}

impl From<Session> for SessionFile {
    fn from(session: Session) -> Self {
        Self(session)
    }
}

impl SessionFile {
    /// Clears the global session file
    pub async fn clear() -> io::Result<()> {
        tokio::fs::remove_file(SESSION_PATH.as_path()).await
    }

    /// Returns true if the global session file exists
    pub fn exists() -> bool {
        SESSION_PATH.exists()
    }

    /// Saves a session to the global session file
    pub async fn save(&self) -> io::Result<()> {
        // Ensure our cache directory exists
        let cache_dir = PROJECT_DIRS.cache_dir();
        tokio::fs::create_dir_all(cache_dir).await?;

        self.save_to(SESSION_PATH.as_path()).await
    }

    /// Saves a session to to a file at the specified path
    pub async fn save_to(&self, path: impl AsRef<Path>) -> io::Result<()> {
        tokio::fs::write(path.as_ref(), self.0.to_unprotected_string().await?).await
    }

    /// Loads a session from the global session file
    pub async fn load() -> io::Result<Self> {
        Self::load_from(SESSION_PATH.as_path()).await
    }

    /// Loads a session from a file at the specified path
    pub async fn load_from(path: impl AsRef<Path>) -> io::Result<Self> {
        let text = tokio::fs::read_to_string(path.as_ref()).await?;

        Ok(Self(text.parse()?))
    }
}
