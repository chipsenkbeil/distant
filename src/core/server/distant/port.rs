use derive_more::{Display, Error};
use std::{
    net::{IpAddr, SocketAddr},
    str::FromStr,
};

/// Represents some range of ports
#[derive(Clone, Debug, Display, PartialEq, Eq)]
#[display(
    fmt = "{}{}",
    start,
    "end.as_ref().map(|end| format!(\"[:{}]\", end)).unwrap_or_default()"
)]
pub struct PortRange {
    pub start: u16,
    pub end: Option<u16>,
}

impl PortRange {
    /// Builds a collection of `SocketAddr` instances from the port range and given ip address
    pub fn make_socket_addrs(&self, addr: impl Into<IpAddr>) -> Vec<SocketAddr> {
        let mut socket_addrs = Vec::new();
        let addr = addr.into();

        for port in self.start..=self.end.unwrap_or(self.start) {
            socket_addrs.push(SocketAddr::from((addr, port)));
        }

        socket_addrs
    }
}

#[derive(Copy, Clone, Debug, Display, Error, PartialEq, Eq)]
pub enum PortRangeParseError {
    InvalidPort,
    MissingPort,
}

impl FromStr for PortRange {
    type Err = PortRangeParseError;

    /// Parses PORT into single range or PORT1:PORTN into full range
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut tokens = s.trim().split(':');
        let start = tokens
            .next()
            .ok_or(PortRangeParseError::MissingPort)?
            .parse::<u16>()
            .map_err(|_| PortRangeParseError::InvalidPort)?;
        let end = if let Some(token) = tokens.next() {
            Some(
                token
                    .parse::<u16>()
                    .map_err(|_| PortRangeParseError::InvalidPort)?,
            )
        } else {
            None
        };

        if tokens.next().is_some() {
            return Err(PortRangeParseError::InvalidPort);
        }

        Ok(Self { start, end })
    }
}
