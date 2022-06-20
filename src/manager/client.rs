use super::{ManagerRequest, ManagerResponse};
use distant_core::net::{
    router, Auth, Client, Codec, FramedTransport, HeapAuthServer, IntoSplit, RawTransport, Request,
    Response,
};
use std::io;

const INBOUND_CAPACITY: usize = 10000;
const OUTBOUND_CAPACITY: usize = 10000;

router!(DistantManagerClientRouter {
    auth_transport: Auth => Auth,
    manager_transport: Request<ManagerRequest> => Response<ManagerResponse>,
});

/// Represents a client that can connect to a remote distant manager
pub struct DistantManagerClient {
    auth: HeapAuthServer,
    client: Client<ManagerRequest, ManagerResponse>,
}

impl DistantManagerClient {
    /// Initializes a client using the provided framed transport
    pub fn new<T, C>(transport: FramedTransport<T, C>) -> io::Result<Self>
    where
        T: RawTransport + 'static,
        C: Codec + Send + 'static,
    {
        let DistantManagerClientRouter {
            auth_transport,
            manager_transport,
            ..
        } = DistantManagerClientRouter::new(transport, INBOUND_CAPACITY, OUTBOUND_CAPACITY);

        // Initialize our client with manager request/response transport
        let (writer, reader) = manager_transport.into_split();
        let client = Client::new(writer, reader)?;

        // Initialize our auth handler with auth/auth transport
        let auth = HeapAuthServer {
            on_challenge: Box::new(|questions, extra| Vec::new()),
            on_verify: Box::new(|kind, text| false),
            on_info: Box::new(|text| {}),
            on_error: Box::new(|kind, description| {}),
        };

        Ok(Self { auth, client })
    }
}
