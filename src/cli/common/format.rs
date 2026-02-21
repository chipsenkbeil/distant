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
