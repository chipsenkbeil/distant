use std::io;
use std::ops::{Deref, DerefMut};

use log::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::task::JoinHandle;

use crate::net::client::{Client, ClientConfig, UntypedClient};
use crate::net::common::{ConnectionId, FramedTransport, InmemoryTransport, UntypedRequest};
use crate::net::manager::data::{ManagerRequest, ManagerResponse};

/// Represents a raw channel between a manager client and server. Underneath, this routes incoming
/// and outgoing data from a proxied server to an inmemory transport.
pub struct RawChannel {
    transport: FramedTransport<InmemoryTransport>,
    task: JoinHandle<()>,
}

impl RawChannel {
    pub fn abort(&self) {
        self.task.abort();
    }

    /// Consumes this channel, returning a typed client wrapping the transport.
    ///
    /// ### Note
    ///
    /// This does not perform any additional handshakes or authentication. All authentication was
    /// performed during separate connection and this merely wraps an inmemory transport that maps
    /// to the primary connection.
    pub fn into_client<T, U>(self) -> Client<T, U>
    where
        T: Send + Sync + Serialize + 'static,
        U: Send + Sync + DeserializeOwned + 'static,
    {
        Client::spawn_inmemory(
            self.transport,
            ClientConfig::default().with_maximum_silence_duration(),
        )
    }

    /// Consumes this channel, returning an untyped client wrapping the transport.
    ///
    /// ### Note
    ///
    /// This does not perform any additional handshakes or authentication. All authentication was
    /// performed during separate connection and this merely wraps an inmemory transport that maps
    /// to the primary connection.
    pub fn into_untyped_client(self) -> UntypedClient {
        UntypedClient::spawn_inmemory(
            self.transport,
            ClientConfig::default().with_maximum_silence_duration(),
        )
    }

    /// Returns reference to the underlying framed transport.
    pub fn as_framed_transport(&self) -> &FramedTransport<InmemoryTransport> {
        &self.transport
    }

    /// Returns mutable reference to the underlying framed transport.
    pub fn as_mut_framed_transport(&mut self) -> &mut FramedTransport<InmemoryTransport> {
        &mut self.transport
    }

    /// Consumes the channel, returning the underlying framed transport.
    pub fn into_framed_transport(self) -> FramedTransport<InmemoryTransport> {
        self.transport
    }
}

impl Deref for RawChannel {
    type Target = FramedTransport<InmemoryTransport>;

    fn deref(&self) -> &Self::Target {
        &self.transport
    }
}

impl DerefMut for RawChannel {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.transport
    }
}

impl RawChannel {
    pub(super) async fn spawn(
        connection_id: ConnectionId,
        client: &mut Client<ManagerRequest, ManagerResponse>,
    ) -> io::Result<Self> {
        let mut mailbox = client
            .mail(ManagerRequest::OpenChannel { id: connection_id })
            .await?;

        // Wait for the first response, which should be channel confirmation
        let channel_id = match mailbox.next().await {
            Some(response) => match response.payload {
                ManagerResponse::ChannelOpened { id } => Ok(id),
                ManagerResponse::Error { description } => Err(io::Error::other(description)),
                x => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("[Conn {connection_id}] Raw channel open unexpected response: {x:?}"),
                )),
            },
            None => Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                format!("[Conn {connection_id}] Raw channel mailbox aborted"),
            )),
        }?;

        // Spawn our channel proxy transport
        let (mut proxy, transport) = FramedTransport::pair(1);

        let mut manager_channel = client.clone_channel();
        let task = tokio::spawn(async move {
            loop {
                tokio::select! {
                    maybe_response = mailbox.next() => {
                        if maybe_response.is_none() {
                            debug!("[Conn {connection_id} :: Chan {channel_id}] Closing from no more responses");
                            break;
                        }

                        match maybe_response.unwrap().payload {
                            ManagerResponse::Channel { response, .. } => {
                                if let Err(x) = proxy.write_frame(response.to_bytes()).await {
                                    error!(
                                        "[Conn {connection_id} :: Chan {channel_id}] Write response failed: {x}"
                                    );
                                }
                            }
                            ManagerResponse::ChannelClosed { .. } => {
                                break;
                            }
                            _ => continue,
                        }
                    }
                    result = proxy.read_frame() => {
                        match result {
                            Ok(Some(frame)) => {
                                let request = match UntypedRequest::from_slice(frame.as_item()) {
                                    Ok(x) => x.into_owned(),
                                    Err(x) => {
                                        error!("[Conn {connection_id} :: Chan {channel_id}] Parse request failed: {x}");
                                        continue;
                                    }
                                };

                                // NOTE: In this situation, we do not expect a response to this
                                //       request (even if the server sends something back)
                                if let Err(x) = manager_channel
                                    .fire(ManagerRequest::Channel {
                                        id: channel_id,
                                        request,
                                    })
                                    .await
                                {
                                    error!("[Conn {connection_id} :: Chan {channel_id}] Forward failed: {x}");
                                }
                            }
                            Ok(None) => {
                                debug!("[Conn {connection_id} :: Chan {channel_id}] Closing from no more requests");
                                break;
                            }
                            Err(x) => {
                                error!("[Conn {connection_id} :: Chan {channel_id}] Read frame failed: {x}");
                            }
                        }
                    }
                }
            }
        });

        Ok(RawChannel { transport, task })
    }
}

#[cfg(test)]
mod tests {
    //! Tests for RawChannel: accessor methods, abort, Deref/DerefMut, into_client conversions,
    //! and spawn() constructor with success/error/unexpected/abort response paths.

    use super::*;
    use crate::net::client::UntypedClient;
    use crate::net::common::{Connection, Request, Response};

    type ManagerClient = Client<ManagerRequest, ManagerResponse>;

    fn setup() -> (ManagerClient, Connection<InmemoryTransport>) {
        let (client_conn, server_conn) = Connection::pair(100);
        let client = UntypedClient::spawn(client_conn, Default::default()).into_typed_client();
        (client, server_conn)
    }

    fn make_raw_channel() -> RawChannel {
        let (transport, _other) = FramedTransport::pair(1);
        let task = tokio::spawn(async {});
        RawChannel { transport, task }
    }

    // ---- RawChannel accessors ----

    #[test_log::test(tokio::test)]
    async fn abort_cancels_task() {
        let channel = make_raw_channel();
        channel.abort();
        // Give time for the abort to propagate
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        assert!(channel.task.is_finished());
    }

    #[test_log::test(tokio::test)]
    async fn as_framed_transport_returns_reference() {
        let channel = make_raw_channel();
        let _transport: &FramedTransport<InmemoryTransport> = channel.as_framed_transport();
        // Just verify it compiles and returns a reference
    }

    #[test_log::test(tokio::test)]
    async fn as_mut_framed_transport_returns_mutable_reference() {
        let mut channel = make_raw_channel();
        let _transport: &mut FramedTransport<InmemoryTransport> = channel.as_mut_framed_transport();
    }

    #[test_log::test(tokio::test)]
    async fn into_framed_transport_consumes_channel() {
        let channel = make_raw_channel();
        let _transport: FramedTransport<InmemoryTransport> = channel.into_framed_transport();
    }

    #[test_log::test(tokio::test)]
    async fn deref_returns_transport_reference() {
        let channel = make_raw_channel();
        let _transport: &FramedTransport<InmemoryTransport> = &channel;
    }

    #[test_log::test(tokio::test)]
    async fn deref_mut_returns_mutable_transport_reference() {
        let mut channel = make_raw_channel();
        let _transport: &mut FramedTransport<InmemoryTransport> = &mut channel;
    }

    #[test_log::test(tokio::test)]
    async fn into_client_returns_typed_client() {
        let channel = make_raw_channel();
        let _client: Client<String, String> = channel.into_client();
    }

    #[test_log::test(tokio::test)]
    async fn into_untyped_client_returns_untyped_client() {
        let channel = make_raw_channel();
        let _client: UntypedClient = channel.into_untyped_client();
    }

    // ---- RawChannel::spawn ----

    #[test_log::test(tokio::test)]
    async fn spawn_returns_channel_on_successful_channel_opened_response() {
        let (mut client, mut transport) = setup();

        tokio::spawn(async move {
            let request = transport
                .read_frame_as::<Request<ManagerRequest>>()
                .await
                .unwrap()
                .unwrap();

            transport
                .write_frame_for(&Response::new(
                    request.id,
                    ManagerResponse::ChannelOpened { id: 42 },
                ))
                .await
                .unwrap();
        });

        let channel = RawChannel::spawn(123, &mut client).await.unwrap();
        // Verify we got a working channel by checking we can abort it
        channel.abort();
    }

    #[test_log::test(tokio::test)]
    async fn spawn_returns_error_on_error_response() {
        let (mut client, mut transport) = setup();

        tokio::spawn(async move {
            let request = transport
                .read_frame_as::<Request<ManagerRequest>>()
                .await
                .unwrap()
                .unwrap();

            transport
                .write_frame_for(&Response::new(
                    request.id,
                    ManagerResponse::Error {
                        description: "channel open failed".to_string(),
                    },
                ))
                .await
                .unwrap();
        });

        match RawChannel::spawn(123, &mut client).await {
            Err(err) => {
                assert_eq!(err.kind(), io::ErrorKind::Other);
                assert!(err.to_string().contains("channel open failed"));
            }
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    #[test_log::test(tokio::test)]
    async fn spawn_returns_error_on_unexpected_response() {
        let (mut client, mut transport) = setup();

        tokio::spawn(async move {
            let request = transport
                .read_frame_as::<Request<ManagerRequest>>()
                .await
                .unwrap()
                .unwrap();

            transport
                .write_frame_for(&Response::new(request.id, ManagerResponse::Killed))
                .await
                .unwrap();
        });

        match RawChannel::spawn(123, &mut client).await {
            Err(err) => assert_eq!(err.kind(), io::ErrorKind::InvalidData),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    #[test_log::test(tokio::test)]
    async fn spawn_returns_error_on_mailbox_abort() {
        let (mut client, mut transport) = setup();

        tokio::spawn(async move {
            let _request = transport
                .read_frame_as::<Request<ManagerRequest>>()
                .await
                .unwrap()
                .unwrap();

            // Drop transport without sending a response
            drop(transport);
        });

        match RawChannel::spawn(123, &mut client).await {
            Err(err) => assert_eq!(err.kind(), io::ErrorKind::ConnectionAborted),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }
}
