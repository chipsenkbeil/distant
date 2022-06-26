use super::{BoxedConnectHandler, ConnectHandler};
use distant_net::{ServerRef, ServerState};
use std::{collections::HashMap, io, sync::Weak};
use tokio::sync::RwLock;

/// Reference to a distant manager's server instance
pub struct DistantManagerRef {
    /// Mapping of "scheme" -> handler
    pub(crate) handlers: Weak<RwLock<HashMap<String, BoxedConnectHandler>>>,

    pub(crate) inner: Box<dyn ServerRef>,
}

impl DistantManagerRef {
    /// Registers a new [`ConnectHandler`] for the specified scheme (e.g. "distant" or "ssh")
    pub async fn register_connect_handler(
        &self,
        scheme: impl Into<String>,
        handler: impl ConnectHandler + 'static,
    ) -> io::Result<()> {
        let handlers = Weak::upgrade(&self.handlers).ok_or_else(|| {
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
    fn state(&self) -> &ServerState {
        self.inner.state()
    }

    fn is_finished(&self) -> bool {
        self.inner.is_finished()
    }

    fn abort(&self) {
        self.inner.abort();
    }
}
