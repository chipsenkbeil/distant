use crate::DistantManager;
use distant_net::{
    Codec, FramedTransport, IntoSplit, MappedListener, ServerExt, UnixSocketListener,
    UnixSocketServerRef,
};
use std::{io, path::Path};

impl DistantManager {
    pub async fn start_unix_socket<P, C>(self, path: P, codec: C) -> io::Result<UnixSocketServerRef>
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
        let inner = self.start(listener)?;
        Ok(UnixSocketServerRef::new(path, inner))
    }
}
