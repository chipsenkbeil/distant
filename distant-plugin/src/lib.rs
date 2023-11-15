#![doc = include_str!("../README.md")]

#[doc = include_str!("../README.md")]
#[cfg(doctest)]
pub struct ReadmeDoctests;

pub mod api;
pub mod client;
pub mod common;
pub mod handlers;

pub use distant_core_auth as auth;
pub use distant_core_protocol as protocol;

/// Interface to a plugin that can register new handlers for launching and connecting to
/// distant-compatible servers.
pub trait Plugin {
    /// Returns a unique name associated with the plugin.
    fn name(&self) -> &'static str;

    /// Invoked immediately after the plugin is loaded. Used for initialization.
    #[allow(unused_variables)]
    fn on_load(&self, registry: &mut PluginRegistry) {}

    /// Invoked immediately before the plugin is unloaded. Used for deallocation of resources.
    fn on_unload(&self) {}
}

/// Registry that contains various handlers and other information tied to plugins.
#[derive(Default)]
pub struct PluginRegistry {
    /// Names of loaded plugins.
    loaded: Vec<&'static str>,

    /// Launch handlers registered by plugins, keyed by scheme.
    launch_handlers: std::collections::HashMap<String, Box<dyn handlers::LaunchHandler>>,

    /// Connect handlers registered by plugins, keyed by scheme.
    connect_handlers: std::collections::HashMap<String, Box<dyn handlers::ConnectHandler>>,
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a list of plugin names associated with this registry.
    pub fn plugin_names(&self) -> &[&'static str] {
        &self.loaded
    }

    /// Inserts the name of the plugin into the registry. If it already exists, nothing happens.
    pub fn insert_plugin_name(&mut self, name: &'static str) {
        if !self.loaded.contains(&name) {
            self.loaded.push(name);
        }
    }

    /// Returns a reference to the launch handler associated with the `scheme` if one exists.
    pub fn launch_handler(&self, scheme: impl AsRef<str>) -> Option<&dyn handlers::LaunchHandler> {
        self.launch_handlers
            .get(scheme.as_ref())
            .map(|x| x.as_ref())
    }

    /// Inserts a new `handler` for `scheme`. Returns true if successfully inserted, otherwise
    /// false if the scheme is already taken.
    pub fn insert_launch_handler(
        &mut self,
        scheme: impl Into<String>,
        handler: impl handlers::LaunchHandler + 'static,
    ) -> bool {
        use std::collections::hash_map::Entry;

        let scheme = scheme.into();
        if let Entry::Vacant(e) = self.launch_handlers.entry(scheme) {
            e.insert(Box::new(handler));
            true
        } else {
            false
        }
    }

    /// Returns a reference to the connect handler associated with the `scheme` if one exists.
    pub fn connect_handler(
        &self,
        scheme: impl AsRef<str>,
    ) -> Option<&dyn handlers::ConnectHandler> {
        self.connect_handlers
            .get(scheme.as_ref())
            .map(|x| x.as_ref())
    }

    /// Inserts a new `handler` for `scheme`. Returns true if successfully inserted, otherwise
    /// false if the scheme is already taken.
    pub fn insert_connect_handler(
        &mut self,
        scheme: impl Into<String>,
        handler: impl handlers::ConnectHandler + 'static,
    ) -> bool {
        use std::collections::hash_map::Entry;

        let scheme = scheme.into();
        if let Entry::Vacant(e) = self.connect_handlers.entry(scheme) {
            e.insert(Box::new(handler));
            true
        } else {
            false
        }
    }
}

impl std::fmt::Debug for PluginRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginRegistry")
            .field("loaded", &self.loaded)
            .field("launch_handlers", &self.launch_handlers.keys())
            .field("connect_handlers", &self.connect_handlers.keys())
            .finish()
    }
}
