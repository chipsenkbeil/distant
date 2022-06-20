use super::{ManagerRequest, ManagerResponse};
use distant_core::net::{
    router, Auth, AuthClient, AuthServer, IntoSplit, Listener, OneshotListener, Request, Response,
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
    /// Initializes a server using the provided [`SerdeTransport`]
    pub fn new<L, T>(listener: L, config: DistantManagerServerConfig) -> io::Result<Self>
    where
        L: Listener<Output = T>,
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

        Ok(Self { auth, server })
    }
}
