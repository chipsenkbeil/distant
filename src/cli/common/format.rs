use distant_core::net::common::ConnectionId;

/// Format a connection list entry for display in selection prompts.
pub fn format_connection(id: ConnectionId, dest: &str) -> String {
    format!("{id} -> {dest}")
}

#[cfg(test)]
mod tests {
    //! Tests for `format_connection`: verifies human-readable formatting of
    //! `ConnectionId` + destination string.

    use test_log::test;

    use super::*;

    #[test]
    fn format_connection_with_full_uri() {
        let result = format_connection(1, "ssh://user@example.com:22");
        assert_eq!(result, "1 -> ssh://user@example.com:22");
    }

    #[test]
    fn format_connection_with_host_only() {
        let result = format_connection(42, "localhost");
        assert_eq!(result, "42 -> localhost");
    }

    #[test]
    fn format_connection_with_scheme_and_host() {
        let result = format_connection(10, "distant://server.local");
        assert_eq!(result, "10 -> distant://server.local");
    }

    #[test]
    fn format_connection_with_ip_host() {
        let result = format_connection(7, "ssh://root@192.168.1.100:22");
        assert_eq!(result, "7 -> ssh://root@192.168.1.100:22");
    }
}
