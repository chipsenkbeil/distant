use super::data::{
    ConnectionId, ConnectionInfo, ConnectionList, Destination, Extra, ManagerRequest,
    ManagerResponse,
};
use crate::{DistantChannel, DistantClient};
use distant_net::{
    router, Auth, AuthServer, Client, IntoSplit, MpscTransport, OneshotListener, Request, Response,
    ServerExt, ServerRef, UntypedTransportRead, UntypedTransportWrite,
};
use log::*;
use std::{
    collections::{hash_map::Entry, HashMap},
    io,
};
use tokio::task::JoinHandle;

mod config;
pub use config::*;

mod ext;
pub use ext::*;

router!(DistantManagerClientRouter {
    auth_transport: Request<Auth> => Response<Auth>,
    manager_transport: Response<ManagerResponse> => Request<ManagerRequest>,
});

/// Represents a client that can connect to a remote distant manager
pub struct DistantManagerClient {
    auth: Box<dyn ServerRef>,
    client: Client<ManagerRequest, ManagerResponse>,
    distant_clients: HashMap<ConnectionId, ClientHandle>,
}

impl Drop for DistantManagerClient {
    fn drop(&mut self) {
        self.auth.abort();
        self.client.abort();
    }
}

struct ClientHandle {
    client: DistantClient,
    forward_task: JoinHandle<()>,
    mailbox_task: JoinHandle<()>,
}

impl Drop for ClientHandle {
    fn drop(&mut self) {
        self.forward_task.abort();
        self.mailbox_task.abort();
    }
}

impl DistantManagerClient {
    /// Initializes a client using the provided [`UntypedTransport`]
    pub fn new<T>(config: DistantManagerClientConfig, transport: T) -> io::Result<Self>
    where
        T: IntoSplit + 'static,
        T::Read: UntypedTransportRead + 'static,
        T::Write: UntypedTransportWrite + 'static,
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

        Ok(Self {
            auth,
            client,
            distant_clients: HashMap::new(),
        })
    }

    /// Request that the manager launches a new server at the given `destination`
    /// with `extra` being passed for destination-specific details, returning the new
    /// `destination` of the spawned server to connect to
    pub async fn launch(
        &mut self,
        destination: impl Into<Destination>,
        extra: impl Into<Extra>,
    ) -> io::Result<Destination> {
        let destination = Box::new(destination.into());
        let extra = extra.into();
        trace!("launch({}, {})", destination, extra);

        let res = self
            .client
            .send(ManagerRequest::Launch { destination, extra })
            .await?;
        match res.payload {
            ManagerResponse::Launched { destination } => Ok(destination),
            ManagerResponse::Error(x) => Err(x.into()),
            x => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Got unexpected response: {:?}", x),
            )),
        }
    }

    /// Request that the manager establishes a new connection at the given `destination`
    /// with `extra` being passed for destination-specific details
    pub async fn connect(
        &mut self,
        destination: impl Into<Destination>,
        extra: impl Into<Extra>,
    ) -> io::Result<ConnectionId> {
        let destination = Box::new(destination.into());
        let extra = extra.into();
        trace!("connect({}, {})", destination, extra);

        let res = self
            .client
            .send(ManagerRequest::Connect { destination, extra })
            .await?;
        match res.payload {
            ManagerResponse::Connected { id } => Ok(id),
            ManagerResponse::Error(x) => Err(x.into()),
            x => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Got unexpected response: {:?}", x),
            )),
        }
    }

    /// Establishes a channel with the server represented by the `connection_id`,
    /// returning a [`DistantChannel`] acting as the connection
    ///
    /// ### Note
    ///
    /// Multiple calls to open a channel against the same connection will result in
    /// clones of the same [`DistantChannel`] rather than establishing a duplicate
    /// remote connection to the same server
    pub async fn open_channel(
        &mut self,
        connection_id: ConnectionId,
    ) -> io::Result<DistantChannel> {
        trace!("open_channel({})", connection_id);
        match self.distant_clients.entry(connection_id) {
            Entry::Occupied(entry) => Ok(entry.get().client.clone_channel()),
            Entry::Vacant(entry) => {
                let mut mailbox = self
                    .client
                    .mail(ManagerRequest::OpenChannel { id: connection_id })
                    .await?;

                // Wait for the first response, which should be channel confirmation
                let channel_id = match mailbox.next().await {
                    Some(response) => match response.payload {
                        ManagerResponse::ChannelOpened { id } => Ok(id),
                        ManagerResponse::Error(x) => Err(x.into()),
                        x => Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("Got unexpected response: {:?}", x),
                        )),
                    },
                    None => Err(io::Error::new(
                        io::ErrorKind::ConnectionAborted,
                        "open_channel mailbox aborted",
                    )),
                }?;

                // Spawn reader and writer tasks to forward requests and replies
                // using our
                let (t1, t2) = MpscTransport::pair(1);
                let (mut writer, mut reader) = t1.into_split();
                let mailbox_task = tokio::spawn(async move {
                    use distant_net::TypedAsyncWrite;
                    while let Some(response) = mailbox.next().await {
                        match response.payload {
                            ManagerResponse::Channel { response, .. } => {
                                if let Err(x) = writer.write(response).await {
                                    error!("[Conn {}] {}", connection_id, x);
                                }
                            }
                            ManagerResponse::ChannelClosed { .. } => break,
                            _ => continue,
                        }
                    }
                });

                let mut manager_channel = self.client.clone_channel();
                let forward_task = tokio::spawn(async move {
                    use distant_net::TypedAsyncRead;
                    loop {
                        match reader.read().await {
                            Ok(Some(request)) => {
                                // NOTE: In this situation, we do expect a response to this
                                //       request (even if the server sends something back)
                                if let Err(x) = manager_channel
                                    .fire(ManagerRequest::Channel {
                                        id: channel_id,
                                        request,
                                    })
                                    .await
                                {
                                    error!("[Conn {}] {}", connection_id, x);
                                }
                            }
                            Ok(None) => break,
                            Err(x) => {
                                error!("[Conn {}] {}", connection_id, x);
                                continue;
                            }
                        }
                    }
                });

                let (writer, reader) = t2.into_split();
                let client = DistantClient::new(writer, reader)?;
                let channel = client.clone_channel();
                entry.insert(ClientHandle {
                    client,
                    forward_task,
                    mailbox_task,
                });
                Ok(channel)
            }
        }
    }

    /// Retrieves information about a specific connection
    pub async fn info(&mut self, id: ConnectionId) -> io::Result<ConnectionInfo> {
        trace!("info({})", id);
        let res = self.client.send(ManagerRequest::Info { id }).await?;
        match res.payload {
            ManagerResponse::Info(info) => Ok(info),
            ManagerResponse::Error(x) => Err(x.into()),
            x => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Got unexpected response: {:?}", x),
            )),
        }
    }

    /// Kills the specified connection
    pub async fn kill(&mut self, id: ConnectionId) -> io::Result<()> {
        trace!("kill({})", id);
        let res = self.client.send(ManagerRequest::Kill { id }).await?;
        match res.payload {
            ManagerResponse::Killed => Ok(()),
            ManagerResponse::Error(x) => Err(x.into()),
            x => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Got unexpected response: {:?}", x),
            )),
        }
    }

    /// Retrieves a list of active connections
    pub async fn list(&mut self) -> io::Result<ConnectionList> {
        trace!("list()");
        let res = self.client.send(ManagerRequest::List).await?;
        match res.payload {
            ManagerResponse::List(list) => Ok(list),
            ManagerResponse::Error(x) => Err(x.into()),
            x => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Got unexpected response: {:?}", x),
            )),
        }
    }

    /// Requests that the manager shuts down
    pub async fn shutdown(&mut self) -> io::Result<()> {
        trace!("shutdown()");
        let res = self.client.send(ManagerRequest::Shutdown).await?;
        match res.payload {
            ManagerResponse::Shutdown => Ok(()),
            ManagerResponse::Error(x) => Err(x.into()),
            x => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Got unexpected response: {:?}", x),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Error, ErrorKind};
    use distant_net::{
        FramedTransport, InmemoryTransport, PlainCodec, UntypedTransportRead, UntypedTransportWrite,
    };

    fn setup() -> (
        DistantManagerClient,
        FramedTransport<InmemoryTransport, PlainCodec>,
    ) {
        let (t1, t2) = FramedTransport::pair(100);
        let client =
            DistantManagerClient::new(DistantManagerClientConfig::with_empty_prompts(), t1)
                .unwrap();
        (client, t2)
    }

    #[inline]
    fn test_error() -> Error {
        Error {
            kind: ErrorKind::Interrupted,
            description: "test error".to_string(),
        }
    }

    #[inline]
    fn test_io_error() -> io::Error {
        test_error().into()
    }

    #[tokio::test]
    async fn connect_should_report_error_if_receives_error_response() {
        let (mut client, mut transport) = setup();

        tokio::spawn(async move {
            let request = transport
                .read::<Request<ManagerRequest>>()
                .await
                .unwrap()
                .unwrap();

            transport
                .write(Response::new(
                    request.id,
                    ManagerResponse::Error(test_error()),
                ))
                .await
                .unwrap();
        });

        let err = client
            .connect(
                "scheme://host".parse::<Destination>().unwrap(),
                "key=value".parse::<Extra>().unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.kind(), test_io_error().kind());
        assert_eq!(err.to_string(), test_io_error().to_string());
    }

    #[tokio::test]
    async fn connect_should_report_error_if_receives_unexpected_response() {
        let (mut client, mut transport) = setup();

        tokio::spawn(async move {
            let request = transport
                .read::<Request<ManagerRequest>>()
                .await
                .unwrap()
                .unwrap();

            transport
                .write(Response::new(request.id, ManagerResponse::Shutdown))
                .await
                .unwrap();
        });

        let err = client
            .connect(
                "scheme://host".parse::<Destination>().unwrap(),
                "key=value".parse::<Extra>().unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn connect_should_return_id_from_successful_response() {
        let (mut client, mut transport) = setup();

        let expected_id = 999;
        tokio::spawn(async move {
            let request = transport
                .read::<Request<ManagerRequest>>()
                .await
                .unwrap()
                .unwrap();

            transport
                .write(Response::new(
                    request.id,
                    ManagerResponse::Connected { id: expected_id },
                ))
                .await
                .unwrap();
        });

        let id = client
            .connect(
                "scheme://host".parse::<Destination>().unwrap(),
                "key=value".parse::<Extra>().unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(id, expected_id);
    }

    #[tokio::test]
    async fn info_should_report_error_if_receives_error_response() {
        let (mut client, mut transport) = setup();

        tokio::spawn(async move {
            let request = transport
                .read::<Request<ManagerRequest>>()
                .await
                .unwrap()
                .unwrap();

            transport
                .write(Response::new(
                    request.id,
                    ManagerResponse::Error(test_error()),
                ))
                .await
                .unwrap();
        });

        let err = client.info(123).await.unwrap_err();
        assert_eq!(err.kind(), test_io_error().kind());
        assert_eq!(err.to_string(), test_io_error().to_string());
    }

    #[tokio::test]
    async fn info_should_report_error_if_receives_unexpected_response() {
        let (mut client, mut transport) = setup();

        tokio::spawn(async move {
            let request = transport
                .read::<Request<ManagerRequest>>()
                .await
                .unwrap()
                .unwrap();

            transport
                .write(Response::new(request.id, ManagerResponse::Shutdown))
                .await
                .unwrap();
        });

        let err = client.info(123).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn info_should_return_connection_info_from_successful_response() {
        let (mut client, mut transport) = setup();

        tokio::spawn(async move {
            let request = transport
                .read::<Request<ManagerRequest>>()
                .await
                .unwrap()
                .unwrap();

            let info = ConnectionInfo {
                id: 123,
                destination: "scheme://host".parse::<Destination>().unwrap(),
                extra: "key=value".parse::<Extra>().unwrap(),
            };

            transport
                .write(Response::new(request.id, ManagerResponse::Info(info)))
                .await
                .unwrap();
        });

        let info = client.info(123).await.unwrap();
        assert_eq!(info.id, 123);
        assert_eq!(
            info.destination,
            "scheme://host".parse::<Destination>().unwrap()
        );
        assert_eq!(info.extra, "key=value".parse::<Extra>().unwrap());
    }

    #[tokio::test]
    async fn list_should_report_error_if_receives_error_response() {
        let (mut client, mut transport) = setup();

        tokio::spawn(async move {
            let request = transport
                .read::<Request<ManagerRequest>>()
                .await
                .unwrap()
                .unwrap();

            transport
                .write(Response::new(
                    request.id,
                    ManagerResponse::Error(test_error()),
                ))
                .await
                .unwrap();
        });

        let err = client.list().await.unwrap_err();
        assert_eq!(err.kind(), test_io_error().kind());
        assert_eq!(err.to_string(), test_io_error().to_string());
    }

    #[tokio::test]
    async fn list_should_report_error_if_receives_unexpected_response() {
        let (mut client, mut transport) = setup();

        tokio::spawn(async move {
            let request = transport
                .read::<Request<ManagerRequest>>()
                .await
                .unwrap()
                .unwrap();

            transport
                .write(Response::new(request.id, ManagerResponse::Shutdown))
                .await
                .unwrap();
        });

        let err = client.list().await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn list_should_return_connection_list_from_successful_response() {
        let (mut client, mut transport) = setup();

        tokio::spawn(async move {
            let request = transport
                .read::<Request<ManagerRequest>>()
                .await
                .unwrap()
                .unwrap();

            let mut list = ConnectionList::new();
            list.insert(123, "scheme://host".parse::<Destination>().unwrap());

            transport
                .write(Response::new(request.id, ManagerResponse::List(list)))
                .await
                .unwrap();
        });

        let list = client.list().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(
            list.get(&123).expect("Connection list missing item"),
            &"scheme://host".parse::<Destination>().unwrap()
        );
    }

    #[tokio::test]
    async fn kill_should_report_error_if_receives_error_response() {
        let (mut client, mut transport) = setup();

        tokio::spawn(async move {
            let request = transport
                .read::<Request<ManagerRequest>>()
                .await
                .unwrap()
                .unwrap();

            transport
                .write(Response::new(
                    request.id,
                    ManagerResponse::Error(test_error()),
                ))
                .await
                .unwrap();
        });

        let err = client.kill(123).await.unwrap_err();
        assert_eq!(err.kind(), test_io_error().kind());
        assert_eq!(err.to_string(), test_io_error().to_string());
    }

    #[tokio::test]
    async fn kill_should_report_error_if_receives_unexpected_response() {
        let (mut client, mut transport) = setup();

        tokio::spawn(async move {
            let request = transport
                .read::<Request<ManagerRequest>>()
                .await
                .unwrap()
                .unwrap();

            transport
                .write(Response::new(request.id, ManagerResponse::Shutdown))
                .await
                .unwrap();
        });

        let err = client.kill(123).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn kill_should_return_success_from_successful_response() {
        let (mut client, mut transport) = setup();

        tokio::spawn(async move {
            let request = transport
                .read::<Request<ManagerRequest>>()
                .await
                .unwrap()
                .unwrap();

            transport
                .write(Response::new(request.id, ManagerResponse::Killed))
                .await
                .unwrap();
        });

        client.kill(123).await.unwrap();
    }

    #[tokio::test]
    async fn shutdown_should_report_error_if_receives_error_response() {
        let (mut client, mut transport) = setup();

        tokio::spawn(async move {
            let request = transport
                .read::<Request<ManagerRequest>>()
                .await
                .unwrap()
                .unwrap();

            transport
                .write(Response::new(
                    request.id,
                    ManagerResponse::Connected { id: 0 },
                ))
                .await
                .unwrap();
        });

        let err = client.shutdown().await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn shutdown_should_report_error_if_receives_unexpected_response() {
        let (mut client, mut transport) = setup();

        tokio::spawn(async move {
            let request = transport
                .read::<Request<ManagerRequest>>()
                .await
                .unwrap()
                .unwrap();

            transport
                .write(Response::new(
                    request.id,
                    ManagerResponse::Error(test_error()),
                ))
                .await
                .unwrap();
        });

        let err = client.shutdown().await.unwrap_err();
        assert_eq!(err.kind(), test_io_error().kind());
        assert_eq!(err.to_string(), test_io_error().to_string());
    }

    #[tokio::test]
    async fn shutdown_should_return_success_from_successful_response() {
        let (mut client, mut transport) = setup();

        tokio::spawn(async move {
            let request = transport
                .read::<Request<ManagerRequest>>()
                .await
                .unwrap()
                .unwrap();

            transport
                .write(Response::new(request.id, ManagerResponse::Shutdown))
                .await
                .unwrap();
        });

        client.shutdown().await.unwrap();
    }
}
