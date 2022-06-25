use super::data::{
    ChannelKind, ConnectionInfo, ConnectionList, Destination, Extra, ManagerRequest,
    ManagerResponse,
};
use crate::{DistantMsg, DistantRequestData, DistantResponseData};
use distant_net::{
    router, Auth, AuthServer, Client, IntoSplit, OneshotListener, Request, Response, ServerExt,
    ServerRef, UntypedTransportRead, UntypedTransportWrite,
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
            ManagerResponse::Connected { id } => Ok(id),
            ManagerResponse::Error(x) => Err(x.into()),
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
            .send(ManagerRequest::OpenChannel {
                id,
                kind: ChannelKind::SingleResponse,
                payload,
            })
            .await?;
        match res.payload {
            ManagerResponse::Channel { payload, .. } => match payload {
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

    /// Same as `Self::send`, but specifically for single requests
    pub async fn send_single(
        &mut self,
        id: usize,
        payload: impl Into<DistantRequestData>,
    ) -> io::Result<DistantResponseData> {
        match self.send(id, DistantMsg::Single(payload.into())).await? {
            DistantMsg::Single(x) => Ok(x),
            DistantMsg::Batch(_) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Got batch response for a single request",
            )),
        }
    }

    /// Same as `Self::send`, but specifically for batch requests
    pub async fn send_batch(
        &mut self,
        id: usize,
        payload: impl Into<Vec<DistantRequestData>>,
    ) -> io::Result<Vec<DistantResponseData>> {
        match self.send(id, DistantMsg::Batch(payload.into())).await? {
            DistantMsg::Batch(x) => Ok(x),
            DistantMsg::Single(_) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Got single response for a batch request",
            )),
        }
    }

    /// Retrieves information about a specific connection
    pub async fn info(&mut self, id: usize) -> io::Result<ConnectionInfo> {
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
    pub async fn kill(&mut self, id: usize) -> io::Result<()> {
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
    async fn send_should_report_error_if_receives_batch_response_for_single_request() {
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
                    ManagerResponse::Channel {
                        id: 456,
                        payload: DistantMsg::Batch(vec![DistantResponseData::Ok]),
                    },
                ))
                .await
                .unwrap();
        });

        let err = client
            .send(123, DistantMsg::Single(DistantRequestData::SystemInfo {}))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn send_should_report_error_if_receives_single_response_for_batch_request() {
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
                    ManagerResponse::Channel {
                        id: 456,
                        payload: DistantMsg::Single(DistantResponseData::Ok),
                    },
                ))
                .await
                .unwrap();
        });

        let err = client
            .send(
                123,
                DistantMsg::Batch(vec![DistantRequestData::SystemInfo {}]),
            )
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn send_should_report_error_if_receives_unexpected_response() {
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
            .send(123, DistantMsg::Single(DistantRequestData::SystemInfo {}))
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn send_should_return_single_response_for_single_request() {
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
                    ManagerResponse::Channel {
                        id: 456,
                        payload: DistantMsg::Single(DistantResponseData::Ok),
                    },
                ))
                .await
                .unwrap();
        });

        let response = client
            .send(123, DistantMsg::Single(DistantRequestData::SystemInfo {}))
            .await
            .unwrap();
        match response {
            DistantMsg::Single(DistantResponseData::Ok) => (),
            x => panic!("Got unexpected response: {:?}", x),
        }
    }

    #[tokio::test]
    async fn send_should_return_batch_response_for_batch_request() {
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
                    ManagerResponse::Channel {
                        id: 456,
                        payload: DistantMsg::Batch(vec![DistantResponseData::Ok]),
                    },
                ))
                .await
                .unwrap();
        });

        let response = client
            .send(
                123,
                DistantMsg::Batch(vec![DistantRequestData::SystemInfo {}]),
            )
            .await
            .unwrap();
        match response {
            DistantMsg::Batch(payloads) => {
                assert_eq!(payloads.len(), 1);
                assert_eq!(payloads[0], DistantResponseData::Ok);
            }
            x => panic!("Got unexpected response: {:?}", x),
        }
    }

    #[tokio::test]
    async fn send_single_should_report_error_if_receives_batch_response() {
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
                    ManagerResponse::Channel {
                        id: 456,
                        payload: DistantMsg::Batch(vec![DistantResponseData::Ok]),
                    },
                ))
                .await
                .unwrap();
        });

        let err = client
            .send_single(123, DistantRequestData::SystemInfo {})
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn send_single_should_report_error_if_receives_unexpected_response() {
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
            .send_single(123, DistantRequestData::SystemInfo {})
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn send_single_should_return_single_response() {
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
                    ManagerResponse::Channel {
                        id: 456,
                        payload: DistantMsg::Single(DistantResponseData::Ok),
                    },
                ))
                .await
                .unwrap();
        });

        let response = client
            .send_single(123, DistantRequestData::SystemInfo {})
            .await
            .unwrap();
        match response {
            DistantResponseData::Ok => (),
            x => panic!("Got unexpected response: {:?}", x),
        }
    }

    #[tokio::test]
    async fn send_batch_should_report_error_if_receives_single_response() {
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
                    ManagerResponse::Channel {
                        id: 456,
                        payload: DistantMsg::Single(DistantResponseData::Ok),
                    },
                ))
                .await
                .unwrap();
        });

        let err = client
            .send_batch(123, vec![DistantRequestData::SystemInfo {}])
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn send_batch_should_report_error_if_receives_unexpected_response() {
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
            .send_batch(123, vec![DistantRequestData::SystemInfo {}])
            .await
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[tokio::test]
    async fn send_batch_should_return_batch_response() {
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
                    ManagerResponse::Channel {
                        id: 456,
                        payload: DistantMsg::Batch(vec![DistantResponseData::Ok]),
                    },
                ))
                .await
                .unwrap();
        });

        let payloads = client
            .send_batch(123, vec![DistantRequestData::SystemInfo {}])
            .await
            .unwrap();
        assert_eq!(payloads.len(), 1);
        assert_eq!(payloads[0], DistantResponseData::Ok);
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

        let _ = client.kill(123).await.unwrap();
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

        let _ = client.shutdown().await.unwrap();
    }
}
