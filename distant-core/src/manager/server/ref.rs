use super::{BoxedConnectHandler, BoxedLaunchHandler, ConnectHandler, LaunchHandler};
use distant_net::server::ServerRef;
use std::{collections::HashMap, io, sync::Weak};
use tokio::sync::RwLock;

/// Reference to a distant manager's server instance
pub struct DistantManagerRef {
    /// Mapping of "scheme" -> handler
    pub(crate) launch_handlers: Weak<RwLock<HashMap<String, BoxedLaunchHandler>>>,

    /// Mapping of "scheme" -> handler
    pub(crate) connect_handlers: Weak<RwLock<HashMap<String, BoxedConnectHandler>>>,

    pub(crate) inner: Box<dyn ServerRef>,
}

impl DistantManagerRef {
    /// Registers a new [`LaunchHandler`] for the specified scheme (e.g. "distant" or "ssh")
    pub async fn register_launch_handler(
        &self,
        scheme: impl Into<String>,
        handler: impl LaunchHandler + 'static,
    ) -> io::Result<()> {
        let handlers = Weak::upgrade(&self.launch_handlers).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Other,
                "Handler reference is no longer available",
            )
        })?;

        handlers
            .write()
            .await
            .insert(scheme.into(), Box::new(handler));

        Ok(())
    }

    /// Registers a new [`ConnectHandler`] for the specified scheme (e.g. "distant" or "ssh")
    pub async fn register_connect_handler(
        &self,
        scheme: impl Into<String>,
        handler: impl ConnectHandler + 'static,
    ) -> io::Result<()> {
        let handlers = Weak::upgrade(&self.connect_handlers).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Other,
                "Handler reference is no longer available",
            )
        })?;

        handlers
            .write()
            .await
            .insert(scheme.into(), Box::new(handler));

        Ok(())
    }
}

impl ServerRef for DistantManagerRef {
    fn is_finished(&self) -> bool {
        self.inner.is_finished()
    }

    fn abort(&self) {
        self.inner.abort();
    }
}
