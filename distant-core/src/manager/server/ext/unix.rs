use crate::{DistantManager, DistantManagerConfig};
use distant_net::{
    Codec, FramedTransport, IntoSplit, MappedListener, UnixSocketListener, UnixSocketServerRef,
};
use std::{io, path::Path};

impl DistantManager {
    /// Start a new server using the specified path as a unix socket
    pub async fn start_unix_socket<P, C>(
        config: DistantManagerConfig,
        path: P,
        codec: C,
    ) -> io::Result<UnixSocketServerRef>
    where
        P: AsRef<Path> + Send,
        C: Codec + Send + Sync + 'static,
    {
        let listener = UnixSocketListener::bind(path)?;
        let path = listener.path().to_path_buf();

        let listener = MappedListener::new(listener, move |transport| {
            let transport = FramedTransport::new(transport, codec.clone());
            transport.into_split()
        });
        let inner = DistantManager::start(config, listener)?;
        Ok(UnixSocketServerRef::new(path, Box::new(inner)))
    }
}
