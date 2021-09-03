use crate::net::{SecretKey, UnprotectedToHexKey};
use derive_more::{Display, Error};
use std::{
    env,
    net::{IpAddr, SocketAddr},
    ops::Deref,
    path::{Path, PathBuf},
    str::FromStr,
};
use tokio::{io, net::lookup_host};

#[derive(Debug, PartialEq, Eq)]
pub struct SessionInfo {
    pub host: String,
    pub port: u16,
    pub auth_key: SecretKey,
}

#[derive(Copy, Clone, Debug, Display, Error, PartialEq, Eq)]
pub enum SessionInfoParseError {
    #[display(fmt = "Prefix of string is invalid")]
    BadPrefix,

    #[display(fmt = "Bad hex key for session")]
    BadHexKey,

    #[display(fmt = "Invalid key for session")]
    InvalidKey,

    #[display(fmt = "Invalid port for session")]
    InvalidPort,

    #[display(fmt = "Missing address for session")]
    MissingAddr,

    #[display(fmt = "Missing key for session")]
    MissingKey,

    #[display(fmt = "Missing port for session")]
    MissingPort,
}

impl From<SessionInfoParseError> for io::Error {
    fn from(x: SessionInfoParseError) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, x)
    }
}

impl FromStr for SessionInfo {
    type Err = SessionInfoParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut tokens = s.split(' ').take(5);

        // First, validate that we have the appropriate prefix
        if tokens.next().ok_or(SessionInfoParseError::BadPrefix)? != "DISTANT" {
            return Err(SessionInfoParseError::BadPrefix);
        }
        if tokens.next().ok_or(SessionInfoParseError::BadPrefix)? != "DATA" {
            return Err(SessionInfoParseError::BadPrefix);
        }

        // Second, load up the address without parsing it
        let host = tokens
            .next()
            .ok_or(SessionInfoParseError::MissingAddr)?
            .trim()
            .to_string();

        // Third, load up the port and parse it into a number
        let port = tokens
            .next()
            .ok_or(SessionInfoParseError::MissingPort)?
            .trim()
            .parse::<u16>()
            .map_err(|_| SessionInfoParseError::InvalidPort)?;

        // Fourth, load up the key and convert it back into a secret key from a hex slice
        let auth_key = SecretKey::from_slice(
            &hex::decode(
                tokens
                    .next()
                    .ok_or(SessionInfoParseError::MissingKey)?
                    .trim(),
            )
            .map_err(|_| SessionInfoParseError::BadHexKey)?,
        )
        .map_err(|_| SessionInfoParseError::InvalidKey)?;

        Ok(SessionInfo {
            host,
            port,
            auth_key,
        })
    }
}

impl SessionInfo {
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

    /// Consumes the session and returns the auth key
    pub fn into_auth_key(self) -> SecretKey {
        self.auth_key
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

    /// Converts to unprotected string that exposes the auth key in the form of
    /// `DISTANT DATA <host> <port> <auth key>`
    pub fn to_unprotected_string(&self) -> String {
        format!(
            "DISTANT DATA {} {} {}",
            self.host,
            self.port,
            self.auth_key.unprotected_to_hex_key()
        )
    }
}

/// Provides operations related to working with a session that is disk-based
pub struct SessionInfoFile {
    path: PathBuf,
    session: SessionInfo,
}

impl AsRef<Path> for SessionInfoFile {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

impl AsRef<SessionInfo> for SessionInfoFile {
    fn as_ref(&self) -> &SessionInfo {
        self.as_session()
    }
}

impl Deref for SessionInfoFile {
    type Target = SessionInfo;

    fn deref(&self) -> &Self::Target {
        &self.session
    }
}

impl From<SessionInfoFile> for SessionInfo {
    fn from(sf: SessionInfoFile) -> Self {
        sf.session
    }
}

impl SessionInfoFile {
    /// Creates a new inmemory pointer to a session and its file
    pub fn new(path: impl Into<PathBuf>, session: SessionInfo) -> Self {
        Self {
            path: path.into(),
            session,
        }
    }

    /// Returns a reference to the path to the session file
    pub fn as_path(&self) -> &Path {
        self.path.as_path()
    }

    /// Returns a reference to the session
    pub fn as_session(&self) -> &SessionInfo {
        &self.session
    }

    /// Saves a session by overwriting its current
    pub async fn save(&self) -> io::Result<()> {
        self.save_to(self.as_path(), true).await
    }

    /// Saves a session to to a file at the specified path
    ///
    /// If all is true, will create all directories leading up to file's location
    pub async fn save_to(&self, path: impl AsRef<Path>, all: bool) -> io::Result<()> {
        if all {
            if let Some(dir) = path.as_ref().parent() {
                tokio::fs::create_dir_all(dir).await?;
            }
        }

        tokio::fs::write(path.as_ref(), self.session.to_unprotected_string()).await
    }

    /// Loads a session from a file at the specified path
    pub async fn load_from(path: impl AsRef<Path>) -> io::Result<Self> {
        let text = tokio::fs::read_to_string(path.as_ref()).await?;

        Ok(Self {
            path: path.as_ref().to_path_buf(),
            session: text.parse()?,
        })
    }
}
