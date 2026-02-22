use std::future::Future;
use std::io;
use std::pin::Pin;

use crate::auth::Authenticator;
use crate::net::client::UntypedClient;
use crate::net::common::{Destination, Map};

mod process;
pub use process::ProcessPlugin;

/// Single interface for all backends (built-in and external).
///
/// Plugins handle both launching and connecting to servers. A plugin declares one or more URI
/// schemes it supports; the manager routes requests to the matching plugin based on scheme.
///
/// Use `Arc<dyn Plugin>` (not `Box`) so a multi-scheme plugin can be the same instance
/// registered for multiple scheme keys in the manager's routing table.
pub trait Plugin: Send + Sync {
    /// Human-readable name for this plugin (e.g. "ssh", "docker").
    /// Used in logging, error messages, and as the default scheme if `schemes()` is not overridden.
    fn name(&self) -> &str;

    /// URI schemes this plugin handles (e.g. `["ssh"]` or `["docker", "docker-compose"]`).
    /// Defaults to a single scheme matching `name()`.
    fn schemes(&self) -> Vec<String> {
        vec![self.name().to_string()]
    }

    /// Connect to an existing server, return a client.
    fn connect<'a>(
        &'a self,
        destination: &'a Destination,
        options: &'a Map,
        authenticator: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<UntypedClient>> + Send + 'a>>;

    /// Launch a server at destination, return connection info.
    /// Not all plugins support launch — default returns Unsupported error.
    fn launch<'a>(
        &'a self,
        _destination: &'a Destination,
        _options: &'a Map,
        _authenticator: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<Destination>> + Send + 'a>> {
        Box::pin(async {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "launch not supported",
            ))
        })
    }
}
