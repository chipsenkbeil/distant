use super::ConnectHandler;
use distant_net::{ServerRef, ServerState};
use std::{collections::HashMap, io, sync::Weak};
use tokio::sync::RwLock;

/// Reference to a distant manager's server instance
pub struct DistantManagerServerRef {
    /// Mapping of "scheme" -> handler
    pub(crate) connect_handlers:
        Weak<RwLock<HashMap<String, Box<dyn ConnectHandler + Send + Sync>>>>,

    pub(crate) inner: Box<dyn ServerRef>,
}

impl DistantManagerServerRef {
    /// Registers a new [`ConnectHandler`] for the specified scheme (e.g. "distant" or "ssh")
    pub async fn register_connect_handler(
        &self,
        scheme: impl Into<String>,
        handler: impl ConnectHandler + Send + Sync + 'static,
    ) -> io::Result<()> {
        let connect_handlers = Weak::upgrade(&self.connect_handlers).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Other,
                "Handler reference is no longer available",
            )
        })?;

        connect_handlers
            .write()
            .await
            .insert(scheme.into(), Box::new(handler));

        Ok(())
    }
}

impl ServerRef for DistantManagerServerRef {
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
