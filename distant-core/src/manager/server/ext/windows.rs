use crate::{DistantManager, DistantManagerConfig};
use distant_net::{
    common::{Codec, FramedTransport,  MappedListener, WindowsPipeListener },
    server::WindowsPipeServerRef,
};
use std::{
    ffi::{OsStr, OsString},
    io,
};

impl DistantManager {
    /// Start a new server at the specified address via `\\.\pipe\{name}` using the given codec
    pub async fn start_local_named_pipe<N, C>(
        config: DistantManagerConfig,
        name: N,
        codec: C,
    ) -> io::Result<WindowsPipeServerRef>
    where
        Self: Sized,
        N: AsRef<OsStr> + Send,
        C: Codec + Send + Sync + 'static,
    {
        let mut addr = OsString::from(r"\\.\pipe\");
        addr.push(name.as_ref());
        Self::start_named_pipe(config, addr, codec).await
    }

    /// Start a new server at the specified pipe address using the given codec
    pub async fn start_named_pipe<A, C>(
        config: DistantManagerConfig,
        addr: A,
        codec: C,
    ) -> io::Result<WindowsPipeServerRef>
    where
        A: AsRef<OsStr> + Send,
        C: Codec + Send + Sync + 'static,
    {
        let a = addr.as_ref();
        let listener = WindowsPipeListener::bind(a)?;
        let addr = listener.addr().to_os_string();

        let listener = MappedListener::new(listener, move |transport| {
            let transport = FramedTransport::new(transport, codec.clone());
            transport.into_split()
        });
        let inner = DistantManager::start(config, listener)?;
        Ok(WindowsPipeServerRef::new(addr, Box::new(inner)))
    }
}
