use crate::{PROJECT_DIRS, SESSION_PATH};
use derive_more::{Display, Error, From};
use orion::aead::SecretKey;
use std::net::{IpAddr, SocketAddr};
use tokio::{io, net::lookup_host};

#[derive(Debug, Display, Error, From)]
pub enum SessionError {
    #[display(fmt = "Bad hex key for session")]
    BadSessionHexKey,

    #[display(fmt = "Invalid address for session")]
    InvalidSessionAddr,

    #[display(fmt = "Invalid key for session")]
    InvalidSessionKey,

    #[display(fmt = "Invalid port for session")]
    InvalidSessionPort,

    IoError(io::Error),

    #[display(fmt = "Missing address for session")]
    MissingSessionAddr,

    #[display(fmt = "Missing key for session")]
    MissingSessionKey,

    #[display(fmt = "Missing port for session")]
    MissingSessionPort,

    #[display(fmt = "No session file: {:?}", SESSION_PATH.as_path())]
    NoSessionFile,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Session {
    pub host: String,
    pub port: u16,
    pub auth_key: SecretKey,
}

impl Session {
    /// Returns a string representing the secret key as hex
    pub fn to_unprotected_hex_auth_key(&self) -> String {
        hex::encode(self.auth_key.unprotected_as_bytes())
    }

    /// Returns the ip address associated with the session based on the host
    pub async fn to_ip_addr(&self) -> io::Result<IpAddr> {
        let addr = match self.host.parse::<IpAddr>() {
            Ok(addr) => addr,
            Err(_) => lookup_host((self.host.as_str(), self.port))
                .await?
                .next()
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, SessionError::InvalidSessionAddr)
                })?
                .ip(),
        };

        Ok(addr)
    }

    /// Returns socket address associated with the session
    pub async fn to_socket_addr(&self) -> io::Result<SocketAddr> {
        let addr = self.to_ip_addr().await?;
        Ok(SocketAddr::from((addr, self.port)))
    }

    /// Clears the global session file
    pub async fn clear() -> io::Result<()> {
        tokio::fs::remove_file(SESSION_PATH.as_path()).await
    }

    /// Returns true if a session is available
    pub fn exists() -> bool {
        SESSION_PATH.exists()
    }

    /// Saves a session to disk
    pub async fn save(&self) -> io::Result<()> {
        let key_hex_str = self.to_unprotected_hex_auth_key();

        // Ensure our cache directory exists
        let cache_dir = PROJECT_DIRS.cache_dir();
        tokio::fs::create_dir_all(cache_dir).await?;

        // Write our session file
        let addr = self.to_ip_addr().await?;
        tokio::fs::write(
            SESSION_PATH.as_path(),
            format!("{} {} {}", addr, self.port, key_hex_str),
        )
        .await?;

        Ok(())
    }

    /// Loads a session's information into memory
    pub async fn load() -> Result<Self, SessionError> {
        let text = tokio::fs::read_to_string(SESSION_PATH.as_path())
            .await
            .map_err(|_| SessionError::NoSessionFile)?;
        let mut tokens = text.split(' ').take(3);

        // First, load up the address without parsing it
        let host = tokens
            .next()
            .ok_or(SessionError::MissingSessionAddr)?
            .trim()
            .to_string();

        // Second, load up the port and parse it into a number
        let port = tokens
            .next()
            .ok_or(SessionError::MissingSessionPort)?
            .trim()
            .parse::<u16>()
            .map_err(|_| SessionError::InvalidSessionPort)?;

        // Third, load up the key and convert it back into a secret key from a hex slice
        let auth_key = SecretKey::from_slice(
            &hex::decode(tokens.next().ok_or(SessionError::MissingSessionKey)?.trim())
                .map_err(|_| SessionError::BadSessionHexKey)?,
        )
        .map_err(|_| SessionError::InvalidSessionKey)?;

        Ok(Session {
            host,
            port,
            auth_key,
        })
    }
}
