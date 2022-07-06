use crate::config::NetworkConfig;
use distant_core::{net::PlainCodec, DistantManager, DistantManagerConfig, DistantManagerRef};
use std::io;

pub struct Manager {
    config: DistantManagerConfig,
    network: NetworkConfig,
}

impl Manager {
    pub fn new(config: DistantManagerConfig, network: NetworkConfig) -> Self {
        Self { config, network }
    }

    /// Begin listening on the network interface specified within [`NetworkConfig`]
    pub async fn listen(self) -> io::Result<DistantManagerRef> {
        #[cfg(unix)]
        {
            let boxed_ref = DistantManager::start_unix_socket(
                self.config,
                self.network.unix_socket_path_or_default(),
                PlainCodec,
            )
            .await?
            .into_inner()
            .into_boxed_server_ref()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Got wrong server ref"))?;

            Ok(*boxed_ref)
        }

        #[cfg(windows)]
        {
            let boxed_ref = DistantManager::start_local_named_pipe(
                self.config,
                self.network.windows_pipe_name_or_default(),
                PlainCodec,
            )
            .await?
            .into_inner()
            .into_boxed_server_ref()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Got wrong server ref"))?;

            Ok(*boxed_ref)
        }
    }
}
