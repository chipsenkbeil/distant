use super::data::{
    ConnectionInfo, ConnectionList, Destination, Extra, ManagerRequest, ManagerResponse,
};
use crate::{DistantMsg, DistantRequestData, DistantResponseData};
use distant_net::{
    router, Auth, AuthServer, Client, IntoSplit, OneshotListener, Request, Response, ServerExt,
    ServerRef, UntypedTransport,
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

impl Drop for DistantManagerClient {
    fn drop(&mut self) {
        self.auth.abort();
        self.client.abort();
    }
}

impl DistantManagerClient {
    /// Initializes a client using the provided [`SerdeTransport`]
    pub fn new<T>(transport: T, config: DistantManagerClientConfig) -> io::Result<Self>
    where
        T: UntypedTransport + 'static,
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

    /// Request that the manager establishes a new connection at the given `destination`
    /// with `extra` being passed for destination-specific details
    pub async fn connect(
        &mut self,
        destination: impl Into<Destination>,
        extra: impl Into<Extra>,
    ) -> io::Result<usize> {
        let destination = Box::new(destination.into());
        let extra = extra.into();
        let res = self
            .client
            .send(ManagerRequest::Connect { destination, extra })
            .await?;
        match res.payload {
            ManagerResponse::Connected(id) => Ok(id),
            x => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Got unexpected response: {:?}", x),
            )),
        }
    }

    /// Sends an arbitrary request to the connection with the specified `id`
    pub async fn send(
        &mut self,
        id: usize,
        payload: impl Into<DistantMsg<DistantRequestData>>,
    ) -> io::Result<DistantMsg<DistantResponseData>> {
        let payload = payload.into();
        let is_batch = payload.is_batch();
        let res = self
            .client
            .send(ManagerRequest::Request { id, payload })
            .await?;
        match res.payload {
            ManagerResponse::Response(payload) => match payload {
                DistantMsg::Single(_) if is_batch => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Expected batch response, but got single payload",
                )),
                DistantMsg::Batch(_) if !is_batch => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Expected single response, but got batch payload",
                )),
                x => Ok(x),
            },
            x => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Got unexpected response: {:?}", x),
            )),
        }
    }

    /// Retrieves information about a specific connection
    pub async fn info(&mut self, id: usize) -> io::Result<ConnectionInfo> {
        let res = self.client.send(ManagerRequest::Info { id }).await?;
        match res.payload {
            ManagerResponse::Info(info) => Ok(info),
            x => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Got unexpected response: {:?}", x),
            )),
        }
    }

    /// Kills the specified connection
    pub async fn kill(&mut self, id: usize) -> io::Result<()> {
        let res = self.client.send(ManagerRequest::Kill { id }).await?;
        match res.payload {
            ManagerResponse::Killed => Ok(()),
            x => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Got unexpected response: {:?}", x),
            )),
        }
    }

    /// Retrieves a list of active connections
    pub async fn list(&mut self) -> io::Result<ConnectionList> {
        let res = self.client.send(ManagerRequest::List).await?;
        match res.payload {
            ManagerResponse::List(list) => Ok(list),
            x => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Got unexpected response: {:?}", x),
            )),
        }
    }

    /// Requests that the manager shuts down
    pub async fn shutdown(&mut self) -> io::Result<()> {
        let res = self.client.send(ManagerRequest::Shutdown).await?;
        match res.payload {
            ManagerResponse::Shutdown => Ok(()),
            x => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Got unexpected response: {:?}", x),
            )),
        }
    }
}
