use derive_more::Display;
use serde::{Deserialize, Serialize};
use std::{
    net::{IpAddr, SocketAddr},
    ops::RangeInclusive,
    str::FromStr,
};

/// Represents some range of ports
#[derive(Copy, Clone, Debug, Display, PartialEq, Eq, Serialize, Deserialize)]
#[display(
    fmt = "{}{}",
    start,
    "end.as_ref().map(|end| format!(\":{}\", end)).unwrap_or_default()"
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

        for port in self {
            socket_addrs.push(SocketAddr::from((addr, port)));
        }

        socket_addrs
    }
}

impl From<u16> for PortRange {
    fn from(port: u16) -> Self {
        Self {
            start: port,
            end: None,
        }
    }
}

impl From<RangeInclusive<u16>> for PortRange {
    fn from(r: RangeInclusive<u16>) -> Self {
        let (start, end) = r.into_inner();
        Self {
            start,
            end: Some(end),
        }
    }
}

impl<'a> IntoIterator for &'a PortRange {
    type Item = u16;
    type IntoIter = RangeInclusive<u16>;

    fn into_iter(self) -> Self::IntoIter {
        self.start..=self.end.unwrap_or(self.start)
    }
}

impl IntoIterator for PortRange {
    type Item = u16;
    type IntoIter = RangeInclusive<u16>;

    fn into_iter(self) -> Self::IntoIter {
        self.start..=self.end.unwrap_or(self.start)
    }
}

impl FromStr for PortRange {
    type Err = std::num::ParseIntError;

    /// Parses PORT into single range or PORT1:PORTN into full range
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.find(':') {
            Some(idx) if idx + 1 < s.len() => Ok(Self {
                start: s[..idx].parse()?,
                end: Some(s[(idx + 1)..].parse()?),
            }),
            _ => Ok(Self {
                start: s.parse()?,
                end: None,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_should_properly_reflect_port_range() {
        let p = PortRange {
            start: 100,
            end: None,
        };
        assert_eq!(p.to_string(), "100");

        let p = PortRange {
            start: 100,
            end: Some(200),
        };
        assert_eq!(p.to_string(), "100:200");
    }

    #[test]
    fn from_range_inclusive_should_map_to_port_range() {
        let p = PortRange::from(100..=200);
        assert_eq!(p.start, 100);
        assert_eq!(p.end, Some(200));
    }

    #[test]
    fn into_iterator_should_support_port_range() {
        let p = PortRange {
            start: 1,
            end: None,
        };
        assert_eq!((&p).into_iter().collect::<Vec<u16>>(), vec![1]);
        assert_eq!(p.into_iter().collect::<Vec<u16>>(), vec![1]);

        let p = PortRange {
            start: 1,
            end: Some(3),
        };
        assert_eq!((&p).into_iter().collect::<Vec<u16>>(), vec![1, 2, 3]);
        assert_eq!(p.into_iter().collect::<Vec<u16>>(), vec![1, 2, 3]);
    }

    #[test]
    fn make_socket_addrs_should_produce_a_socket_addr_per_port() {
        let ip_addr = "127.0.0.1".parse::<IpAddr>().unwrap();

        let p = PortRange {
            start: 1,
            end: None,
        };
        assert_eq!(
            p.make_socket_addrs(ip_addr),
            vec![SocketAddr::new(ip_addr, 1)]
        );

        let p = PortRange {
            start: 1,
            end: Some(3),
        };
        assert_eq!(
            p.make_socket_addrs(ip_addr),
            vec![
                SocketAddr::new(ip_addr, 1),
                SocketAddr::new(ip_addr, 2),
                SocketAddr::new(ip_addr, 3),
            ]
        );
    }

    #[test]
    fn parse_should_fail_if_not_starting_with_number() {
        assert!("100a".parse::<PortRange>().is_err());
    }

    #[test]
    fn parse_should_fail_if_provided_end_port_that_is_not_a_number() {
        assert!("100:200a".parse::<PortRange>().is_err());
    }

    #[test]
    fn parse_should_be_able_to_properly_read_in_port_range() {
        let p: PortRange = "100".parse().unwrap();
        assert_eq!(
            p,
            PortRange {
                start: 100,
                end: None
            }
        );

        let p: PortRange = "100:200".parse().unwrap();
        assert_eq!(
            p,
            PortRange {
                start: 100,
                end: Some(200)
            }
        );
    }
}
