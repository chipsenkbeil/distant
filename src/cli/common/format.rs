use distant_core::net::common::{ConnectionId, Destination};

/// Format a connection list entry for display in selection prompts.
pub fn format_connection(id: ConnectionId, dest: &Destination) -> String {
    let scheme = dest
        .scheme
        .as_ref()
        .map(|s| format!("{s}://"))
        .unwrap_or_default();
    let user = dest
        .username
        .as_ref()
        .map(|u| format!("{u}@"))
        .unwrap_or_default();
    let port = dest.port.map(|p| format!(":{p}")).unwrap_or_default();
    format!("{id} -> {scheme}{user}{}{port}", dest.host)
}

#[cfg(test)]
mod tests {
    use distant_core::net::common::Host;
    use test_log::test;

    use super::*;

    #[test]
    fn format_connection_with_all_fields() {
        let dest = Destination {
            scheme: Some("ssh".to_string()),
            username: Some("user".to_string()),
            password: None,
            host: Host::Name("example.com".to_string()),
            port: Some(22),
        };
        let result = format_connection(1, &dest);
        assert_eq!(result, "1 -> ssh://user@example.com:22");
    }

    #[test]
    fn format_connection_with_no_optional_fields() {
        let dest = Destination {
            scheme: None,
            username: None,
            password: None,
            host: Host::Name("localhost".to_string()),
            port: None,
        };
        let result = format_connection(42, &dest);
        assert_eq!(result, "42 -> localhost");
    }

    #[test]
    fn format_connection_with_scheme_only() {
        let dest = Destination {
            scheme: Some("distant".to_string()),
            username: None,
            password: None,
            host: Host::Name("server.local".to_string()),
            port: None,
        };
        let result = format_connection(10, &dest);
        assert_eq!(result, "10 -> distant://server.local");
    }

    #[test]
    fn format_connection_with_username_only() {
        let dest = Destination {
            scheme: None,
            username: Some("admin".to_string()),
            password: None,
            host: Host::Name("host.example.com".to_string()),
            port: None,
        };
        let result = format_connection(5, &dest);
        assert_eq!(result, "5 -> admin@host.example.com");
    }

    #[test]
    fn format_connection_with_port_only() {
        let dest = Destination {
            scheme: None,
            username: None,
            password: None,
            host: Host::Name("myhost".to_string()),
            port: Some(8080),
        };
        let result = format_connection(3, &dest);
        assert_eq!(result, "3 -> myhost:8080");
    }

    #[test]
    fn format_connection_with_ip_host() {
        use std::net::Ipv4Addr;
        let dest = Destination {
            scheme: Some("ssh".to_string()),
            username: Some("root".to_string()),
            password: None,
            host: Host::Ipv4(Ipv4Addr::new(192, 168, 1, 100)),
            port: Some(22),
        };
        let result = format_connection(7, &dest);
        assert_eq!(result, "7 -> ssh://root@192.168.1.100:22");
    }
}
