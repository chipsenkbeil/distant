use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::ops::RangeInclusive;
use std::str::FromStr;

use derive_more::Display;
use serde::{Deserialize, Serialize, de};

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
    //! Tests for PortRange: parsing, display, iteration, serde, is_ephemeral, make_socket_addrs
    //! with IPv4/IPv6, and boundary conditions (port 0, max u16, inverted ranges).

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

    // -----------------------------------------------------------------------
    // is_ephemeral
    // -----------------------------------------------------------------------

    #[test]
    fn is_ephemeral_returns_true_for_port_zero_with_no_end() {
        assert!(PortRange::EPHEMERAL.is_ephemeral());
        assert!(PortRange::single(0).is_ephemeral());
    }

    #[test]
    fn is_ephemeral_returns_false_for_nonzero_port() {
        assert!(!PortRange::single(1).is_ephemeral());
        assert!(!PortRange::single(8080).is_ephemeral());
    }

    #[test]
    fn is_ephemeral_returns_false_when_start_is_zero_but_end_is_some() {
        let p = PortRange {
            start: 0,
            end: Some(0),
        };
        assert!(!p.is_ephemeral());
    }

    // -----------------------------------------------------------------------
    // make_socket_addrs with IPv6
    // -----------------------------------------------------------------------

    #[test]
    fn make_socket_addrs_should_work_with_ipv6() {
        use std::net::Ipv6Addr;
        let ip = Ipv6Addr::LOCALHOST;
        let p = PortRange {
            start: 80,
            end: Some(82),
        };
        let addrs = p.make_socket_addrs(ip);
        assert_eq!(addrs.len(), 3);
        assert_eq!(addrs[0], SocketAddr::new(IpAddr::V6(ip), 80));
        assert_eq!(addrs[1], SocketAddr::new(IpAddr::V6(ip), 81));
        assert_eq!(addrs[2], SocketAddr::new(IpAddr::V6(ip), 82));
    }

    #[test]
    fn make_socket_addrs_should_work_with_ipv4addr_directly() {
        use std::net::Ipv4Addr;
        let ip = Ipv4Addr::new(192, 168, 1, 1);
        let p = PortRange::single(443);
        let addrs = p.make_socket_addrs(ip);
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0], SocketAddr::new(IpAddr::V4(ip), 443));
    }

    // -----------------------------------------------------------------------
    // FromStr edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn parse_should_fail_for_trailing_colon() {
        // "100:" has a colon at index 3 but idx + 1 == s.len(), so it falls
        // through to the single-port parse path where "100:" fails.
        assert!("100:".parse::<PortRange>().is_err());
    }

    #[test]
    fn parse_should_fail_for_empty_string() {
        assert!("".parse::<PortRange>().is_err());
    }

    #[test]
    fn parse_should_fail_for_non_numeric_string() {
        assert!("abc".parse::<PortRange>().is_err());
    }

    #[test]
    fn parse_should_fail_for_negative_number() {
        assert!("-1".parse::<PortRange>().is_err());
    }

    #[test]
    fn parse_should_fail_for_value_exceeding_u16() {
        assert!("70000".parse::<PortRange>().is_err());
    }

    #[test]
    fn parse_should_fail_for_range_with_end_exceeding_u16() {
        assert!("1:70000".parse::<PortRange>().is_err());
    }

    #[test]
    fn parse_should_fail_for_range_with_non_numeric_start() {
        assert!("abc:100".parse::<PortRange>().is_err());
    }

    #[test]
    fn parse_should_succeed_for_port_zero() {
        let p: PortRange = "0".parse().unwrap();
        assert_eq!(
            p,
            PortRange {
                start: 0,
                end: None
            }
        );
    }

    #[test]
    fn parse_should_succeed_for_max_u16() {
        let p: PortRange = "65535".parse().unwrap();
        assert_eq!(p.start, 65535);
        assert_eq!(p.end, None);
    }

    #[test]
    fn parse_should_succeed_for_range_with_same_start_and_end() {
        let p: PortRange = "80:80".parse().unwrap();
        assert_eq!(p.start, 80);
        assert_eq!(p.end, Some(80));
    }

    // -----------------------------------------------------------------------
    // From<u16>
    // -----------------------------------------------------------------------

    #[test]
    fn from_u16_should_create_single_port_range() {
        let p = PortRange::from(8080u16);
        assert_eq!(p.start, 8080);
        assert_eq!(p.end, None);
    }

    // -----------------------------------------------------------------------
    // Iterator edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn into_iterator_should_yield_empty_when_start_exceeds_end() {
        // This tests the RangeInclusive behavior when start > end
        let p = PortRange {
            start: 5,
            end: Some(3),
        };
        let ports: Vec<u16> = p.into_iter().collect();
        assert!(ports.is_empty());
    }

    #[test]
    fn into_iterator_should_yield_single_element_for_same_start_and_end() {
        let p = PortRange {
            start: 42,
            end: Some(42),
        };
        let ports: Vec<u16> = p.into_iter().collect();
        assert_eq!(ports, [42]);
    }

    // -----------------------------------------------------------------------
    // Serde with various numeric types
    // -----------------------------------------------------------------------

    #[test]
    fn deserialize_should_handle_u8_via_msgpack() {
        // msgpack encodes small numbers as u8; we use serde_json's Number
        // which always uses visit_u64, so instead we test the visitor directly
        // by round-tripping through formats that produce different integer types.

        // JSON always calls visit_u64 for positive integers, so use that:
        let p: PortRange = serde_json::from_str("255").unwrap();
        assert_eq!(p.start, 255);
        assert_eq!(p.end, None);
    }

    #[test]
    fn deserialize_should_fail_for_negative_json_number() {
        // JSON -1 calls visit_i64; i64 -1 cannot convert to u16
        assert!(serde_json::from_str::<PortRange>("-1").is_err());
    }

    #[test]
    fn deserialize_should_fail_for_number_exceeding_u16_max() {
        assert!(serde_json::from_str::<PortRange>("70000").is_err());
    }

    #[test]
    fn deserialize_should_fail_for_invalid_string_format() {
        assert!(serde_json::from_str::<PortRange>("\"abc\"").is_err());
    }

    // -----------------------------------------------------------------------
    // Display round-trip through FromStr
    // -----------------------------------------------------------------------

    #[test]
    fn display_and_parse_should_roundtrip_single() {
        let original = PortRange::single(443);
        let s = original.to_string();
        let parsed: PortRange = s.parse().unwrap();
        assert_eq!(original, parsed);
    }

    #[test]
    fn display_and_parse_should_roundtrip_range() {
        let original = PortRange {
            start: 8000,
            end: Some(9000),
        };
        let s = original.to_string();
        let parsed: PortRange = s.parse().unwrap();
        assert_eq!(original, parsed);
    }

    // -----------------------------------------------------------------------
    // Serde round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn serde_roundtrip_single_port() {
        let original = PortRange::single(22);
        let json = serde_json::to_string(&original).unwrap();
        let restored: PortRange = serde_json::from_str(&json).unwrap();
        assert_eq!(original, restored);
    }

    #[test]
    fn serde_roundtrip_port_range() {
        let original = PortRange {
            start: 3000,
            end: Some(4000),
        };
        let json = serde_json::to_string(&original).unwrap();
        let restored: PortRange = serde_json::from_str(&json).unwrap();
        assert_eq!(original, restored);
    }
}
