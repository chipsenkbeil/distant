use crate::{
    config::NetworkConfig,
    paths::{global as global_paths, user as user_paths},
};
use distant_core::{net::PlainCodec, DistantManager, DistantManagerConfig, DistantManagerRef};
use log::*;
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
        let user = self.config.user;

        #[cfg(unix)]
        {
            let socket_path = self.network.unix_socket.as_deref().unwrap_or({
                if user {
                    user_paths::UNIX_SOCKET_PATH.as_path()
                } else {
                    global_paths::UNIX_SOCKET_PATH.as_path()
                }
            });

            let boxed_ref = DistantManager::start_unix_socket_with_permissions(
                self.config,
                socket_path,
                PlainCodec,
                self.network
                    .unix_socket_permissions
                    .unwrap_or_default()
                    .into_mode(),
            )
            .await?
            .into_inner()
            .into_boxed_server_ref()
            .map_err(|_| io::Error::new(io::ErrorKind::Other, "Got wrong server ref"))?;

            info!("Manager listening using unix socket @ {:?}", socket_path);
            Ok(*boxed_ref)
        }

        #[cfg(windows)]
        {
            let pipe_name = self.network.windows_pipe.as_deref().unwrap_or(if user {
                user_paths::WINDOWS_PIPE_NAME.as_str()
            } else {
                global_paths::WINDOWS_PIPE_NAME.as_str()
            });
            let boxed_ref =
                DistantManager::start_local_named_pipe(self.config, pipe_name, PlainCodec)
                    .await?
                    .into_inner()
                    .into_boxed_server_ref()
                    .map_err(|_| io::Error::new(io::ErrorKind::Other, "Got wrong server ref"))?;

            info!("Manager listening using local named pipe @ {:?}", pipe_name);
            Ok(*boxed_ref)
        }
    }
}
