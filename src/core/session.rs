use derive_more::{Display, Error};
use orion::aead::SecretKey;
use std::{
    env,
    net::{IpAddr, SocketAddr},
    ops::Deref,
    path::{Path, PathBuf},
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

    /// Returns a string representing the auth key as hex
    pub fn to_unprotected_hex_auth_key(&self) -> String {
        hex::encode(self.auth_key.unprotected_as_bytes())
    }

    /// Converts to unprotected string that exposes the auth key in the form of
    /// `DISTANT DATA <host> <port> <auth key>`
    pub fn to_unprotected_string(&self) -> String {
        format!(
            "DISTANT DATA {} {} {}",
            self.host,
            self.port,
            self.to_unprotected_hex_auth_key()
        )
    }
}

/// Provides operations related to working with a session that is disk-based
pub struct SessionFile {
    path: PathBuf,
    session: Session,
}

impl AsRef<Path> for SessionFile {
    fn as_ref(&self) -> &Path {
        self.as_path()
    }
}

impl AsRef<Session> for SessionFile {
    fn as_ref(&self) -> &Session {
        self.as_session()
    }
}

impl Deref for SessionFile {
    type Target = Session;

    fn deref(&self) -> &Self::Target {
        &self.session
    }
}

impl From<SessionFile> for Session {
    fn from(sf: SessionFile) -> Self {
        sf.session
    }
}

impl SessionFile {
    /// Creates a new inmemory pointer to a session and its file
    pub fn new(path: impl Into<PathBuf>, session: Session) -> Self {
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
    pub fn as_session(&self) -> &Session {
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
