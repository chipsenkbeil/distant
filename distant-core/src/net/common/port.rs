use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::ops::RangeInclusive;
use std::str::FromStr;

use derive_more::Display;
use serde::{de, Deserialize, Serialize};

/// Represents some range of ports
#[derive(Copy, Clone, Debug, Display, PartialEq, Eq)]
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
    /// Represents an ephemeral port as defined using the port range of 0.
    pub const EPHEMERAL: Self = Self {
        start: 0,
        end: None,
    };

    /// Creates a port range targeting a single `port`.
    #[inline]
    pub fn single(port: u16) -> Self {
        Self {
            start: port,
            end: None,
        }
    }

    /// Builds a collection of `SocketAddr` instances from the port range and given ip address
    pub fn make_socket_addrs(&self, addr: impl Into<IpAddr>) -> Vec<SocketAddr> {
        let mut socket_addrs = Vec::new();
        let addr = addr.into();

        for port in self {
            socket_addrs.push(SocketAddr::from((addr, port)));
        }

        socket_addrs
    }

    /// Returns true if port range represents the ephemeral port.
    #[inline]
    pub fn is_ephemeral(&self) -> bool {
        self == &Self::EPHEMERAL
    }
}

impl From<u16> for PortRange {
    fn from(port: u16) -> Self {
        Self::single(port)
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

impl IntoIterator for &PortRange {
    type IntoIter = RangeInclusive<u16>;
    type Item = u16;

    fn into_iter(self) -> Self::IntoIter {
        self.start..=self.end.unwrap_or(self.start)
    }
}

impl IntoIterator for PortRange {
    type IntoIter = RangeInclusive<u16>;
    type Item = u16;

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

impl Serialize for PortRange {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::ser::Serializer,
    {
        String::serialize(&self.to_string(), serializer)
    }
}

impl<'de> Deserialize<'de> for PortRange {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        struct PortRangeVisitor;
        impl<'de> de::Visitor<'de> for PortRangeVisitor {
            type Value = PortRange;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                write!(formatter, "a port in the form NUMBER or START:END")
            }

            fn visit_str<E>(self, s: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                FromStr::from_str(s).map_err(de::Error::custom)
            }

            fn visit_u8<E>(self, v: u8) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(PortRange {
                    start: v as u16,
                    end: None,
                })
            }

            fn visit_u16<E>(self, v: u16) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(PortRange {
                    start: v,
                    end: None,
                })
            }

            fn visit_u32<E>(self, v: u32) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(PortRange {
                    start: v.try_into().map_err(de::Error::custom)?,
                    end: None,
                })
            }

            fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(PortRange {
                    start: v.try_into().map_err(de::Error::custom)?,
                    end: None,
                })
            }

            fn visit_u128<E>(self, v: u128) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(PortRange {
                    start: v.try_into().map_err(de::Error::custom)?,
                    end: None,
                })
            }

            fn visit_i8<E>(self, v: i8) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(PortRange {
                    start: v.try_into().map_err(de::Error::custom)?,
                    end: None,
                })
            }

            fn visit_i16<E>(self, v: i16) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(PortRange {
                    start: v.try_into().map_err(de::Error::custom)?,
                    end: None,
                })
            }

            fn visit_i32<E>(self, v: i32) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(PortRange {
                    start: v.try_into().map_err(de::Error::custom)?,
                    end: None,
                })
            }

            fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(PortRange {
                    start: v.try_into().map_err(de::Error::custom)?,
                    end: None,
                })
            }

            fn visit_i128<E>(self, v: i128) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(PortRange {
                    start: v.try_into().map_err(de::Error::custom)?,
                    end: None,
                })
            }
        }

        deserializer.deserialize_any(PortRangeVisitor)
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

    #[test]
    fn serialize_should_leverage_tostring() {
        assert_eq!(
            serde_json::to_value(PortRange {
                start: 123,
                end: None,
            })
            .unwrap(),
            serde_json::Value::String("123".to_string())
        );

        assert_eq!(
            serde_json::to_value(PortRange {
                start: 123,
                end: Some(456),
            })
            .unwrap(),
            serde_json::Value::String("123:456".to_string())
        );
    }

    #[test]
    fn deserialize_should_use_single_number_as_start() {
        // Supports parsing numbers
        assert_eq!(
            serde_json::from_str::<PortRange>("123").unwrap(),
            PortRange {
                start: 123,
                end: None
            }
        );
    }

    #[test]
    fn deserialize_should_leverage_fromstr_for_strings() {
        // Supports string number
        assert_eq!(
            serde_json::from_str::<PortRange>("\"123\"").unwrap(),
            PortRange {
                start: 123,
                end: None
            }
        );

        // Supports string start:end
        assert_eq!(
            serde_json::from_str::<PortRange>("\"123:456\"").unwrap(),
            PortRange {
                start: 123,
                end: Some(456)
            }
        );
    }
}
