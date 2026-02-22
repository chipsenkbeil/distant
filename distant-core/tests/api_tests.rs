use std::io;
use std::path::PathBuf;

use distant_core::net::auth::{DummyAuthHandler, Verifier};
use distant_core::net::client::Client as NetClient;
use distant_core::net::common::{InmemoryTransport, OneshotListener, Version};
use distant_core::net::server::{Server, ServerRef};
use distant_core::protocol::PROTOCOL_VERSION;
use distant_core::{Api, ApiServerHandler, ChannelExt, Client, Ctx};

/// Stands up an inmemory client and server using the given api.
async fn setup(api: impl Api + Send + Sync + 'static) -> (Client, ServerRef) {
    let (t1, t2) = InmemoryTransport::pair(100);

    let server = Server::new()
        .handler(ApiServerHandler::new(api))
        .verifier(Verifier::none())
        .version(Version::new(
            PROTOCOL_VERSION.major,
            PROTOCOL_VERSION.minor,
            PROTOCOL_VERSION.patch,
        ))
        .start(OneshotListener::from_value(t2))
        .expect("Failed to start server");

    let client: Client = NetClient::build()
        .auth_handler(DummyAuthHandler)
        .connector(t1)
        .version(Version::new(
            PROTOCOL_VERSION.major,
            PROTOCOL_VERSION.minor,
            PROTOCOL_VERSION.patch,
        ))
        .connect()
        .await
        .expect("Failed to connect to server");

    (client, server)
}

mod single {
    use test_log::test;

    use super::*;

    #[test(tokio::test)]
    async fn should_support_single_request_returning_error() {
        struct TestApi;

        impl Api for TestApi {
            async fn read_file(&self, _ctx: Ctx, _path: PathBuf) -> io::Result<Vec<u8>> {
                Err(io::Error::new(io::ErrorKind::NotFound, "test error"))
            }
        }

        let (mut client, _server) = setup(TestApi).await;

        let error = client.read_file(PathBuf::from("file")).await.unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert_eq!(error.to_string(), "test error");
    }

    #[test(tokio::test)]
    async fn should_support_single_request_returning_success() {
        struct TestApi;

        impl Api for TestApi {
            async fn read_file(&self, _ctx: Ctx, _path: PathBuf) -> io::Result<Vec<u8>> {
                Ok(b"hello world".to_vec())
            }
        }

        let (mut client, _server) = setup(TestApi).await;

        let contents = client.read_file(PathBuf::from("file")).await.unwrap();
        assert_eq!(contents, b"hello world");
    }
}

mod batch_parallel {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use distant_core::net::common::Request;
    use distant_core::protocol::{Msg, Request as RequestPayload};
    use test_log::test;

    use super::*;

    #[test(tokio::test)]
    async fn should_support_multiple_requests_running_in_parallel() {
        struct TestApi;

        impl Api for TestApi {
            async fn read_file(&self, _ctx: Ctx, path: PathBuf) -> io::Result<Vec<u8>> {
                if path.to_str().unwrap() == "slow" {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }

                let time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
                Ok((time.as_millis() as u64).to_be_bytes().to_vec())
            }
        }

        let (mut client, _server) = setup(TestApi).await;

        let request = Request::new(Msg::batch([
            RequestPayload::FileRead {
                path: PathBuf::from("file1"),
            },
            RequestPayload::FileRead {
                path: PathBuf::from("slow"),
            },
            RequestPayload::FileRead {
                path: PathBuf::from("file2"),
            },
        ]));

        let response = client.send(request).await.unwrap();
        let payloads = response.payload.into_batch().unwrap();

        // Collect our times from the reading
        let mut times = Vec::new();
        for payload in payloads {
            match payload {
                distant_core::protocol::Response::Blob { data } => {
                    let mut buf = [0u8; 8];
                    buf.copy_from_slice(&data[..8]);
                    times.push(u64::from_be_bytes(buf));
                }
                x => panic!("Unexpected payload: {x:?}"),
            }
        }

        // Verify that these ran in parallel as the first and third requests should not be
        // over 500 milliseconds apart due to the sleep in the middle!
        let diff = times[0].abs_diff(times[2]);
        assert!(diff <= 500, "Sequential ordering detected");
    }

    #[test(tokio::test)]
    async fn should_run_all_requests_even_if_some_fail() {
        struct TestApi;

        impl Api for TestApi {
            async fn read_file(&self, _ctx: Ctx, path: PathBuf) -> io::Result<Vec<u8>> {
                if path.to_str().unwrap() == "fail" {
                    return Err(io::Error::other("test error"));
                }

                Ok(Vec::new())
            }
        }

        let (mut client, _server) = setup(TestApi).await;

        let request = Request::new(Msg::batch([
            RequestPayload::FileRead {
                path: PathBuf::from("file1"),
            },
            RequestPayload::FileRead {
                path: PathBuf::from("fail"),
            },
            RequestPayload::FileRead {
                path: PathBuf::from("file2"),
            },
        ]));

        let response = client.send(request).await.unwrap();
        let payloads = response.payload.into_batch().unwrap();

        // Should be a success, error, and success
        assert!(
            matches!(payloads[0], distant_core::protocol::Response::Blob { .. }),
            "Unexpected payloads[0]: {:?}",
            payloads[0]
        );
        assert!(
            matches!(
                &payloads[1],
                distant_core::protocol::Response::Error(distant_core::protocol::Error { kind, description })
                if matches!(kind, distant_core::protocol::ErrorKind::Other) && description == "test error"
            ),
            "Unexpected payloads[1]: {:?}",
            payloads[1]
        );
        assert!(
            matches!(payloads[2], distant_core::protocol::Response::Blob { .. }),
            "Unexpected payloads[2]: {:?}",
            payloads[2]
        );
    }
}

mod batch_sequence {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use distant_core::net::common::Request;
    use distant_core::protocol::{Msg, Request as RequestPayload};
    use test_log::test;

    use super::*;

    #[test(tokio::test)]
    async fn should_support_multiple_requests_running_in_sequence() {
        struct TestApi;

        impl Api for TestApi {
            async fn read_file(&self, _ctx: Ctx, path: PathBuf) -> io::Result<Vec<u8>> {
                if path.to_str().unwrap() == "slow" {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }

                let time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
                Ok((time.as_millis() as u64).to_be_bytes().to_vec())
            }
        }

        let (mut client, _server) = setup(TestApi).await;

        let mut request = Request::new(Msg::batch([
            RequestPayload::FileRead {
                path: PathBuf::from("file1"),
            },
            RequestPayload::FileRead {
                path: PathBuf::from("slow"),
            },
            RequestPayload::FileRead {
                path: PathBuf::from("file2"),
            },
        ]));

        // Mark as running in sequence
        request.header.insert("sequence", true);

        let response = client.send(request).await.unwrap();
        let payloads = response.payload.into_batch().unwrap();

        // Collect our times from the reading
        let mut times = Vec::new();
        for payload in payloads {
            match payload {
                distant_core::protocol::Response::Blob { data } => {
                    let mut buf = [0u8; 8];
                    buf.copy_from_slice(&data[..8]);
                    times.push(u64::from_be_bytes(buf));
                }
                x => panic!("Unexpected payload: {x:?}"),
            }
        }

        // Verify that these ran in sequence as the first and third requests should be
        // over 500 milliseconds apart due to the sleep in the middle!
        let diff = times[0].abs_diff(times[2]);
        assert!(diff > 500, "Parallel ordering detected");
    }

    #[test(tokio::test)]
    async fn should_interrupt_any_requests_following_a_failure() {
        struct TestApi;

        impl Api for TestApi {
            async fn read_file(&self, _ctx: Ctx, path: PathBuf) -> io::Result<Vec<u8>> {
                if path.to_str().unwrap() == "fail" {
                    return Err(io::Error::other("test error"));
                }

                Ok(Vec::new())
            }
        }

        let (mut client, _server) = setup(TestApi).await;

        let mut request = Request::new(Msg::batch([
            RequestPayload::FileRead {
                path: PathBuf::from("file1"),
            },
            RequestPayload::FileRead {
                path: PathBuf::from("fail"),
            },
            RequestPayload::FileRead {
                path: PathBuf::from("file2"),
            },
        ]));

        // Mark as running in sequence
        request.header.insert("sequence", true);

        let response = client.send(request).await.unwrap();
        let payloads = response.payload.into_batch().unwrap();

        // Should be a success, error, and interrupt
        assert!(
            matches!(payloads[0], distant_core::protocol::Response::Blob { .. }),
            "Unexpected payloads[0]: {:?}",
            payloads[0]
        );
        assert!(
            matches!(
                &payloads[1],
                distant_core::protocol::Response::Error(distant_core::protocol::Error { kind, description })
                if matches!(kind, distant_core::protocol::ErrorKind::Other) && description == "test error"
            ),
            "Unexpected payloads[1]: {:?}",
            payloads[1]
        );
        assert!(
            matches!(
                &payloads[2],
                distant_core::protocol::Response::Error(distant_core::protocol::Error { kind, .. })
                if matches!(kind, distant_core::protocol::ErrorKind::Interrupted)
            ),
            "Unexpected payloads[2]: {:?}",
            payloads[2]
        );
    }
}
