use anyhow::Context;
use distant_core::net::common::authentication::Verifier;
use distant_core::net::manager::{Config as ManagerConfig, ManagerServer};
use distant_core::net::server::ServerRef;
use log::*;

use crate::constants::{global as global_paths, user as user_paths};
use crate::options::{AccessControl, NetworkSettings};

pub struct Manager {
    pub access: AccessControl,
    pub config: ManagerConfig,
    pub network: NetworkSettings,
}

impl Manager {
    /// Begin listening on the network interface specified within [`NetworkConfig`]
    pub async fn listen(self) -> anyhow::Result<Box<dyn ServerRef>> {
        let user = self.config.user;

        #[cfg(unix)]
        {
            use distant_core::net::common::UnixSocketListener;
            let socket_path = self.network.unix_socket.as_deref().unwrap_or({
                if user {
                    user_paths::UNIX_SOCKET_PATH.as_path()
                } else {
                    global_paths::UNIX_SOCKET_PATH.as_path()
                }
            });

            // Ensure that the path to the socket exists
            if let Some(parent) = socket_path.parent() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .with_context(|| format!("Failed to create socket directory {parent:?}"))?;
            }

            let boxed_ref = ManagerServer::new(self.config)
                .verifier(Verifier::none())
                .start(
                    UnixSocketListener::bind_with_permissions(socket_path, self.access.into_mode())
                        .await?,
                )
                .with_context(|| format!("Failed to start manager at socket {socket_path:?}"))?;

            info!("Manager listening using unix socket @ {:?}", socket_path);
            Ok(boxed_ref)
        }

        #[cfg(windows)]
        {
            use distant_core::net::common::WindowsPipeListener;
            let pipe_name = self.network.windows_pipe.as_deref().unwrap_or(if user {
                user_paths::WINDOWS_PIPE_NAME.as_str()
            } else {
                global_paths::WINDOWS_PIPE_NAME.as_str()
            });

            let boxed_ref = ManagerServer::new(self.config)
                .verifier(Verifier::none())
                .start(WindowsPipeListener::bind_local(pipe_name)?)
                .with_context(|| format!("Failed to start manager at pipe {pipe_name:?}"))?;

            info!("Manager listening using windows pipe @ {:?}", pipe_name);
            Ok(boxed_ref)
        }
    }
}
