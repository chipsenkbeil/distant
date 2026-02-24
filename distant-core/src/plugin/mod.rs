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
            _destination: &'a Destination,
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
            _destination: &'a Destination,
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
        let dest: Destination = "ssh://localhost".parse().unwrap();
        let options = Map::new();
        let mut auth = TestAuthenticator::default();

        let result = plugin.launch(&dest, &options, &mut auth).await;
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
        let dest: Destination = "ssh://localhost".parse().unwrap();
        let options = Map::new();
        let mut auth = TestAuthenticator::default();

        let result = plugin.connect(&dest, &options, &mut auth).await;
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
        let dest: Destination = "ssh://host".parse().unwrap();
        let options = Map::new();
        let mut auth = TestAuthenticator::default();

        let result = plugin.launch(&dest, &options, &mut auth).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Unsupported);
    }

    #[test(tokio::test)]
    async fn arc_dyn_plugin_connect_delegates_to_impl() {
        let plugin: Arc<dyn Plugin> = Arc::new(MockPlugin::new("arctest"));
        let dest: Destination = "ssh://host".parse().unwrap();
        let options = Map::new();
        let mut auth = TestAuthenticator::default();

        let result = plugin.connect(&dest, &options, &mut auth).await;
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
}
