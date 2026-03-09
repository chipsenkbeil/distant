//! # host
//!
//! Ssh host type

use std::fmt;

use wildmatch::WildMatch;

use super::HostParams;

/// Describes the rules to be used for a certain host
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    /// List of hosts for which params are valid. String is string pattern, bool is whether condition is negated
    pub pattern: Vec<HostClause>,
    pub params: HostParams,
}

impl Host {
    pub fn new(pattern: Vec<HostClause>, params: HostParams) -> Self {
        Self { pattern, params }
    }

    /// Returns whether `host` argument intersects the host clauses
    pub fn intersects(&self, host: &str) -> bool {
        let mut has_matched = false;
        for entry in self.pattern.iter() {
            let matches = entry.intersects(host);
            // If the entry is negated and it matches we can stop searching
            if matches && entry.negated {
                return false;
            }
            has_matched |= matches;
        }
        has_matched
    }
}

/// Describes a single clause to match host
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostClause {
    pub pattern: String,
    pub negated: bool,
}

impl fmt::Display for HostClause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.negated {
            write!(f, "!{}", self.pattern)
        } else {
            write!(f, "{}", self.pattern)
        }
    }
}

impl HostClause {
    /// Creates a new `HostClause` from arguments
    pub fn new(pattern: String, negated: bool) -> Self {
        Self { pattern, negated }
    }

    /// Returns whether `host` argument intersects the clause
    pub fn intersects(&self, host: &str) -> bool {
        WildMatch::new(self.pattern.as_str()).matches(host)
    }
}

#[cfg(test)]
mod tests {

    use pretty_assertions::assert_eq;

    use super::*;
    use crate::DefaultAlgorithms;

    #[test]
    fn should_build_host_clause() {
        let clause = HostClause::new("192.168.1.1".to_string(), false);
        assert_eq!(clause.pattern.as_str(), "192.168.1.1");
        assert_eq!(clause.negated, false);
    }

    #[test]
    fn should_intersect_host_clause() {
        let clause = HostClause::new("192.168.*.*".to_string(), false);
        assert!(clause.intersects("192.168.2.30"));
        let clause = HostClause::new("192.168.?0.*".to_string(), false);
        assert!(clause.intersects("192.168.40.28"));
    }

    #[test]
    fn should_not_intersect_host_clause() {
        let clause = HostClause::new("192.168.*.*".to_string(), false);
        assert_eq!(clause.intersects("172.26.104.4"), false);
    }

    #[test]
    fn should_init_host() {
        let host = Host::new(
            vec![HostClause::new("192.168.*.*".to_string(), false)],
            HostParams::new(&DefaultAlgorithms::default()),
        );
        assert_eq!(host.pattern.len(), 1);
    }

    #[test]
    fn should_intersect_clause() {
        let host = Host::new(
            vec![
                HostClause::new("192.168.*.*".to_string(), false),
                HostClause::new("172.26.*.*".to_string(), false),
                HostClause::new("10.8.*.*".to_string(), false),
                HostClause::new("10.8.0.8".to_string(), true),
            ],
            HostParams::new(&DefaultAlgorithms::default()),
        );
        assert!(host.intersects("192.168.1.32"));
        assert!(host.intersects("172.26.104.4"));
        assert!(host.intersects("10.8.0.10"));
    }

    #[test]
    fn should_not_intersect_clause() {
        let host = Host::new(
            vec![
                HostClause::new("192.168.*.*".to_string(), false),
                HostClause::new("172.26.*.*".to_string(), false),
                HostClause::new("10.8.*.*".to_string(), false),
                HostClause::new("10.8.0.8".to_string(), true),
            ],
            HostParams::new(&DefaultAlgorithms::default()),
        );
        assert_eq!(host.intersects("192.169.1.32"), false);
        assert_eq!(host.intersects("172.28.104.4"), false);
        assert_eq!(host.intersects("10.9.0.8"), false);
        assert_eq!(host.intersects("10.8.0.8"), false);
    }

    #[test]
    fn should_display_host_clause() {
        let clause = HostClause::new("192.168.*.*".to_string(), false);
        assert_eq!(clause.to_string(), "192.168.*.*");

        let negated_clause = HostClause::new("192.168.1.1".to_string(), true);
        assert_eq!(negated_clause.to_string(), "!192.168.1.1");
    }

    #[test]
    fn should_not_intersect_with_empty_pattern() {
        let host = Host::new(vec![], HostParams::new(&DefaultAlgorithms::default()));
        assert_eq!(host.intersects("any-host"), false);
    }

    #[test]
    fn should_intersect_with_single_char_wildcard() {
        let clause = HostClause::new("server?".to_string(), false);
        assert!(clause.intersects("server1"));
        assert!(clause.intersects("serverA"));
        assert!(!clause.intersects("server12"));
        assert!(!clause.intersects("server"));
    }

    #[test]
    fn should_intersect_with_only_negated_clauses_after_positive() {
        // A host with positive and negated clauses where negated comes last
        let host = Host::new(
            vec![
                HostClause::new("*.example.com".to_string(), false),
                HostClause::new("secret.example.com".to_string(), true),
            ],
            HostParams::new(&DefaultAlgorithms::default()),
        );
        assert!(host.intersects("www.example.com"));
        assert!(!host.intersects("secret.example.com"));
        assert!(!host.intersects("other.net"));
    }

    #[test]
    fn should_handle_wildcard_at_start() {
        let clause = HostClause::new("*-server".to_string(), false);
        assert!(clause.intersects("prod-server"));
        assert!(clause.intersects("dev-server"));
        assert!(!clause.intersects("server-prod"));
    }

    #[test]
    fn should_handle_wildcard_at_end() {
        let clause = HostClause::new("server-*".to_string(), false);
        assert!(clause.intersects("server-prod"));
        assert!(clause.intersects("server-dev"));
        assert!(!clause.intersects("prod-server"));
    }

    #[test]
    fn should_match_exact_pattern() {
        let clause = HostClause::new("exact-host".to_string(), false);
        assert!(clause.intersects("exact-host"));
        assert!(!clause.intersects("exact-host-extra"));
        assert!(!clause.intersects("prefix-exact-host"));
    }

    #[test]
    fn should_match_universal_wildcard() {
        let clause = HostClause::new("*".to_string(), false);
        assert!(clause.intersects("any-host"));
        assert!(clause.intersects("192.168.1.1"));
        assert!(clause.intersects(""));
    }

    #[test]
    fn should_intersect_negated_clause_returns_true_for_matching_negated() {
        // Test that a negated clause still "intersects" (matches the pattern)
        let clause = HostClause::new("192.168.*.*".to_string(), true);
        assert!(clause.intersects("192.168.1.1")); // intersects returns true for pattern match
    }
}
