use super::{ManagerRequest, ManagerResponse};
use distant_core::net::{
    router, Auth, AuthServer, Client, IntoSplit, OneshotListener, Request, Response,
    SerdeTransport, ServerExt, ServerRef,
};
use std::io;

mod config;
pub use config::*;

router!(DistantManagerClientRouter {
    auth_transport: Request<Auth> => Response<Auth>,
    manager_transport: Response<ManagerResponse> => Request<ManagerRequest>,
});

/// Represents a client that can connect to a remote distant manager
pub struct DistantManagerClient {
    auth: Box<dyn ServerRef>,
    client: Client<ManagerRequest, ManagerResponse>,
}

impl DistantManagerClient {
    /// Initializes a client using the provided [`SerdeTransport`]
    pub fn new<T>(transport: T, config: DistantManagerClientConfig) -> io::Result<Self>
    where
        T: SerdeTransport + 'static,
    {
        let DistantManagerClientRouter {
            auth_transport,
            manager_transport,
            ..
        } = DistantManagerClientRouter::new(transport);

        // Initialize our client with manager request/response transport
        let (writer, reader) = manager_transport.into_split();
        let client = Client::new(writer, reader)?;

        // Initialize our auth handler with auth/auth transport
        let auth = AuthServer {
            on_challenge: config.on_challenge,
            on_verify: config.on_verify,
            on_info: config.on_info,
            on_error: config.on_error,
        }
        .start(OneshotListener::from_value(auth_transport.into_split()))?;

        Ok(Self { auth, client })
    }
}
