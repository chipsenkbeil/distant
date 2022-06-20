use super::{ManagerRequest, ManagerResponse};
use distant_core::net::{
    router, Auth, AuthClient, AuthServer, Client, IntoSplit, OneshotListener, Request, Response,
    SerdeTransport, ServerExt, ServerRef,
};
use std::io;

const INBOUND_CAPACITY: usize = 10000;
const OUTBOUND_CAPACITY: usize = 10000;

mod config;
pub use config::*;

router!(DistantManagerServerRouter {
    auth_transport: Response<Auth> => Request<Auth>,
    manager_transport: Request<ManagerRequest> => Response<ManagerResponse>,
});

/// Represents a server that can connect to a remote distant manager
pub struct DistantManagerServer {
    auth: AuthClient,
    server: Box<dyn ServerRef>,
}

impl DistantManagerServer {
    /// Initializes a server using the provided [`FramedTransport`]
    pub fn new<T>(transport: T, config: DistantManagerServerConfig) -> io::Result<Self>
    where
        T: SerdeTransport + 'static,
    {
        let DistantManagerServerRouter {
            auth_transport,
            manager_transport,
            ..
        } = DistantManagerServerRouter::new(transport);

        // Initialize our server with manager request/response transport
        let (writer, reader) = manager_transport.into_split();
        let server = Server::new(writer, reader)?;

        // Initialize our auth handler with auth/auth transport
        let auth = AuthServer {
            on_challenge: config.on_challenge,
            on_verify: config.on_verify,
            on_info: config.on_info,
            on_error: config.on_error,
        }
        .start(OneshotListener::from_value(auth_transport.into_split()))?;

        Ok(Self { auth, server })
    }
}
