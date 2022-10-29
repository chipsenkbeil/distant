use crate::{DistantManager, DistantManagerConfig};
use distant_net::{
    common::{Codec, FramedTransport, MappedListener, UnixSocketListener},
    server::UnixSocketServerRef,
};
use std::{io, path::Path};

impl DistantManager {
    /// Start a new server using the specified path as a unix socket using default unix socket file
    /// permissions
    pub async fn start_unix_socket<P, C>(
        config: DistantManagerConfig,
        path: P,
        codec: C,
    ) -> io::Result<UnixSocketServerRef>
    where
        P: AsRef<Path> + Send,
        C: Codec + Send + Sync + 'static,
    {
        Self::start_unix_socket_with_permissions(
            config,
            path,
            codec,
            UnixSocketListener::default_unix_socket_file_permissions(),
        )
        .await
    }

    /// Start a new server using the specified path as a unix socket and `mode` as the unix socket
    /// file permissions
    pub async fn start_unix_socket_with_permissions<P, C>(
        config: DistantManagerConfig,
        path: P,
        codec: C,
        mode: u32,
    ) -> io::Result<UnixSocketServerRef>
    where
        P: AsRef<Path> + Send,
        C: Codec + Send + Sync + 'static,
    {
        let listener = UnixSocketListener::bind_with_permissions(path, mode).await?;
        let path = listener.path().to_path_buf();

        let listener = MappedListener::new(listener, move |transport| {
            let transport = FramedTransport::new(transport, codec.clone());
            transport.into_split()
        });
        let inner = DistantManager::start(config, listener)?;
        Ok(UnixSocketServerRef::new(path, Box::new(inner)))
    }
}
