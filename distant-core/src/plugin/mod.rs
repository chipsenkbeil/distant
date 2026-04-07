use std::future::Future;
use std::io;
use std::pin::Pin;

use crate::auth::Authenticator;
use crate::net::client::{ReconnectStrategy, UntypedClient};
use crate::net::common::{Destination, Map};

mod mount;
mod process;

pub use mount::{MountHandle, MountPlugin, MountProbe};
pub use process::ProcessPlugin;

/// Object-safe plugin interface used by the manager.
///
/// Accepts raw destination strings, allowing each plugin to parse destinations according
/// to its own rules. Most plugins parse via [`Destination::from_str`], but plugins with
/// non-standard URI formats (e.g. `docker://ubuntu:22.04` where `:22.04` is an image tag)
/// can implement custom parsing.
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

    /// Connect to an existing server using a raw destination string.
    fn connect<'a>(
        &'a self,
        raw_destination: &'a str,
        options: &'a Map,
        authenticator: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<UntypedClient>> + Send + 'a>>;

    /// Launch a server using a raw destination string, returning connection info.
    /// Not all plugins support launch — default returns Unsupported error.
    fn launch<'a>(
        &'a self,
        _raw_destination: &'a str,
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

    /// Attempt to re-establish a previously connected session.
    ///
    /// Called by the manager when a connection dies. Receives the same
    /// destination and options from the original `connect()` call. The
    /// default returns `Unsupported` (no reconnection capability).
    fn reconnect<'a>(
        &'a self,
        _raw_destination: &'a str,
        _options: &'a Map,
        _authenticator: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<UntypedClient>> + Send + 'a>> {
        Box::pin(async {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "reconnect not supported",
            ))
        })
    }

    /// Reconnection retry strategy for this plugin.
    ///
    /// Returns the strategy the manager should use when orchestrating
    /// reconnection attempts. The default is `Fail` (no automatic
    /// reconnection). Plugins override this to specify backoff behavior.
    fn reconnect_strategy(&self) -> ReconnectStrategy {
        ReconnectStrategy::Fail
    }
}

/// Parses a raw destination string into a core [`Destination`].
///
/// Convenience helper for plugins that use the standard URI-based destination format.
/// Returns an [`io::ErrorKind::InvalidInput`] error with a descriptive message on failure.
pub fn parse_destination(raw: &str) -> io::Result<Destination> {
    raw.parse().map_err(|e: &str| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("Failed to parse destination '{raw}': {e}"),
        )
    })
}

/// Extracts the scheme portion from a raw destination string.
///
/// Returns `Some("docker")` for `"docker://ubuntu:22.04"`, or `None` if no `://` is present.
pub fn extract_scheme(raw: &str) -> Option<&str> {
    raw.split_once("://").map(|(scheme, _)| scheme)
}

#[cfg(test)]
mod tests {
    //! Tests for the Plugin trait: name(), default schemes(), default launch() returning
    //! Unsupported, connect() delegation, and usage through Arc<dyn Plugin>.

    use std::sync::Arc;

    use test_log::test;

    use super::*;
    use crate::auth::TestAuthenticator;

    /// A minimal mock plugin that only implements the required methods,
    /// relying on defaults for `schemes()` and `launch()`.
    struct MockPlugin {
        plugin_name: &'static str,
    }

    impl MockPlugin {
        fn new(name: &'static str) -> Self {
            Self { plugin_name: name }
        }
    }

    impl Plugin for MockPlugin {
        fn name(&self) -> &str {
            self.plugin_name
        }

        fn connect<'a>(
            &'a self,
            _raw_destination: &'a str,
            _options: &'a Map,
            _authenticator: &'a mut dyn Authenticator,
        ) -> Pin<Box<dyn Future<Output = io::Result<UntypedClient>> + Send + 'a>> {
            Box::pin(async { Err(io::Error::other("mock connect not implemented")) })
        }
    }

    /// A mock plugin that overrides `schemes()` to return custom schemes.
    struct CustomSchemesPlugin;

    impl Plugin for CustomSchemesPlugin {
        fn name(&self) -> &str {
            "custom"
        }

        fn schemes(&self) -> Vec<String> {
            vec!["proto-a".to_string(), "proto-b".to_string()]
        }

        fn connect<'a>(
            &'a self,
            _raw_destination: &'a str,
            _options: &'a Map,
            _authenticator: &'a mut dyn Authenticator,
        ) -> Pin<Box<dyn Future<Output = io::Result<UntypedClient>> + Send + 'a>> {
            Box::pin(async { Err(io::Error::other("mock connect not implemented")) })
        }
    }

    // -----------------------------------------------------------------------
    // name()
    // -----------------------------------------------------------------------

    #[test]
    fn name_returns_configured_name() {
        let plugin = MockPlugin::new("ssh");
        assert_eq!(plugin.name(), "ssh");
    }

    #[test]
    fn name_returns_different_configured_name() {
        let plugin = MockPlugin::new("docker");
        assert_eq!(plugin.name(), "docker");
    }

    // -----------------------------------------------------------------------
    // schemes() default implementation
    // -----------------------------------------------------------------------

    #[test]
    fn default_schemes_returns_vec_containing_name() {
        let plugin = MockPlugin::new("myproto");
        let schemes = plugin.schemes();
        assert_eq!(schemes.len(), 1);
        assert_eq!(schemes[0], "myproto");
    }

    #[test]
    fn overridden_schemes_returns_custom_schemes() {
        let plugin = CustomSchemesPlugin;
        let schemes = plugin.schemes();
        assert_eq!(schemes.len(), 2);
        assert_eq!(schemes[0], "proto-a");
        assert_eq!(schemes[1], "proto-b");
    }

    // -----------------------------------------------------------------------
    // launch() default implementation
    // -----------------------------------------------------------------------

    #[test(tokio::test)]
    async fn default_launch_returns_unsupported_error() {
        let plugin = MockPlugin::new("test");
        let options = Map::new();
        let mut auth = TestAuthenticator::default();

        let result = plugin.launch("ssh://localhost", &options, &mut auth).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
        assert_eq!(err.to_string(), "launch not supported");
    }

    // -----------------------------------------------------------------------
    // connect() on mock returns error (exercises the required method path)
    // -----------------------------------------------------------------------

    #[test(tokio::test)]
    async fn mock_connect_returns_error() {
        let plugin = MockPlugin::new("test");
        let options = Map::new();
        let mut auth = TestAuthenticator::default();

        let result = plugin.connect("ssh://localhost", &options, &mut auth).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("mock connect"));
    }

    // -----------------------------------------------------------------------
    // Plugin as trait object via Arc<dyn Plugin>
    // -----------------------------------------------------------------------

    #[test]
    fn plugin_can_be_used_as_arc_dyn_trait_object() {
        let plugin: Arc<dyn Plugin> = Arc::new(MockPlugin::new("remote"));
        assert_eq!(plugin.name(), "remote");
        assert_eq!(plugin.schemes(), vec!["remote".to_string()]);
    }

    #[test(tokio::test)]
    async fn arc_dyn_plugin_launch_returns_unsupported() {
        let plugin: Arc<dyn Plugin> = Arc::new(MockPlugin::new("arctest"));
        let options = Map::new();
        let mut auth = TestAuthenticator::default();

        let result = plugin.launch("ssh://host", &options, &mut auth).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Unsupported);
    }

    #[test(tokio::test)]
    async fn arc_dyn_plugin_connect_delegates_to_impl() {
        let plugin: Arc<dyn Plugin> = Arc::new(MockPlugin::new("arctest"));
        let options = Map::new();
        let mut auth = TestAuthenticator::default();

        let result = plugin.connect("ssh://host", &options, &mut auth).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("mock connect"));
    }

    // -----------------------------------------------------------------------
    // Multiple Arc references to same plugin instance
    // -----------------------------------------------------------------------

    #[test]
    fn multiple_arc_refs_share_same_plugin_instance() {
        let plugin: Arc<dyn Plugin> = Arc::new(MockPlugin::new("shared"));
        let clone1 = Arc::clone(&plugin);
        let clone2 = Arc::clone(&plugin);

        assert_eq!(plugin.name(), "shared");
        assert_eq!(clone1.name(), "shared");
        assert_eq!(clone2.name(), "shared");

        // All references point to the same allocation
        assert_eq!(Arc::strong_count(&plugin), 3);
    }

    #[test(tokio::test)]
    async fn default_reconnect_returns_unsupported_error() {
        let plugin = MockPlugin::new("test");
        let options = Map::new();
        let mut auth = TestAuthenticator::default();

        let result = plugin
            .reconnect("ssh://localhost", &options, &mut auth)
            .await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
        assert_eq!(err.to_string(), "reconnect not supported");
    }

    #[test]
    fn default_reconnect_strategy_returns_fail() {
        let plugin = MockPlugin::new("test");
        let strategy = plugin.reconnect_strategy();
        assert!(strategy.is_fail());
    }

    #[test(tokio::test)]
    async fn arc_dyn_plugin_reconnect_returns_unsupported() {
        let plugin: Arc<dyn Plugin> = Arc::new(MockPlugin::new("arctest"));
        let options = Map::new();
        let mut auth = TestAuthenticator::default();

        let result = plugin.reconnect("ssh://host", &options, &mut auth).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
        assert_eq!(err.to_string(), "reconnect not supported");
    }

    #[test]
    fn arc_dyn_plugin_reconnect_strategy_returns_fail() {
        let plugin: Arc<dyn Plugin> = Arc::new(MockPlugin::new("arctest"));
        let strategy = plugin.reconnect_strategy();
        assert!(strategy.is_fail());
    }

    // -----------------------------------------------------------------------
    // parse_destination helper
    // -----------------------------------------------------------------------

    #[test]
    fn parse_destination_succeeds_for_valid_input() {
        let dest = parse_destination("ssh://host:22").unwrap();
        assert_eq!(dest.scheme.as_deref(), Some("ssh"));
        assert_eq!(dest.port, Some(22));
    }

    #[test]
    fn parse_destination_fails_for_invalid_input() {
        let err = parse_destination("/").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    // -----------------------------------------------------------------------
    // extract_scheme helper
    // -----------------------------------------------------------------------

    #[test]
    fn extract_scheme_returns_scheme_when_present() {
        assert_eq!(extract_scheme("ssh://host"), Some("ssh"));
        assert_eq!(extract_scheme("docker://ubuntu:22.04"), Some("docker"));
    }

    #[test]
    fn extract_scheme_returns_none_when_absent() {
        assert_eq!(extract_scheme("host:22"), None);
        assert_eq!(extract_scheme("localhost"), None);
    }
}
