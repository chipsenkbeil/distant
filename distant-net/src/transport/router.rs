/// Creates a new struct around a [`FramedTransport`](crate::FramedTransport) that routes incoming
/// and outgoing messages to different transports, enabling the ability to transform a singular,
/// framed transport into multiple typed transports that can be combined with
/// [`Client`](crate::Client) and [`Server`](crate::Server) to mix having a variety of clients and
/// servers available on the same underlying [`FramedTransport`](crate::FramedTransport).
///
/// ```no_run
/// use distant_net::router;
///
/// # // To send, the data needs to be serializable
/// # // To receive, the data needs to be deserializable
/// # #[derive(serde::Serialize, serde::Deserialize)]
/// # struct CustomData(u8, u8);
///
/// // Create a router that produces three transports from one:
/// // 1. `Transport<u8, String>` - receives `String` and sends `u8`
/// // 1. `Transport<bool, CustomData>` - receives `CustomData` and sends `bool`
/// // 1. `Transport<Option<String>, u8>` - receives `u8` and sends `Option<String>`
/// router!(TestRouter {
///     one: String => u8,
///     two: CustomData => bool,
///     three: u8 => Option<String>,
/// });
///
/// # let (transport, _) = distant_net::FramedTransport::pair(1);
///
/// let router = TestRouter::new(
///     /* FramedTransport */ transport,
///     /* inbound_buffer  */ 100,
///     /* outbound_buffer */ 100,
/// );
///
/// let one   = router.one;   // MpscTransport<u8, String>
/// let two   = router.two;   // MpscTransport<bool, CustomData>
/// let three = router.three; // MpscTransport<Option<String>, u8>
/// ```
#[macro_export]
macro_rules! router {
    (
        $vis:vis $name:ident {
            $($transport:ident : $res_ty:ty => $req_ty:ty),+ $(,)?
        }
    ) => {
        $crate::paste::paste! {
            #[allow(dead_code)]
            $vis struct $name {
                reader_task: tokio::task::JoinHandle<()>,
                writer_task: tokio::task::JoinHandle<()>,
                $(
                    pub $transport: $crate::MpscTransport<$req_ty, $res_ty>,
                )+
            }

            #[allow(dead_code)]
            impl $name {
                pub fn new<T, C>(
                    transport: $crate::FramedTransport<T, C>,
                    inbound_buffer: usize,
                    outbound_buffer: usize,
                ) -> Self
                where
                    T: $crate::RawTransport + 'static,
                    C: $crate::Codec + Send + 'static,
                {
                    $(
                        let (
                            [<$transport:snake _inbound_tx>],
                            [<$transport:snake _inbound_rx>]
                        ) = tokio::sync::mpsc::channel(inbound_buffer);
                        let (
                            [<$transport:snake _outbound_tx>],
                            mut [<$transport:snake _outbound_rx>]
                        ) = tokio::sync::mpsc::channel(outbound_buffer);
                        let [<$transport:snake>]: $crate::MpscTransport<$req_ty, $res_ty> =
                            $crate::MpscTransport::new(
                                [<$transport:snake _outbound_tx>],
                                [<$transport:snake _inbound_rx>]
                            );
                    )+

                    #[derive(serde::Deserialize)]
                    #[serde(untagged)]
                    enum [<$name:camel In>] {
                        $([<$transport:camel>]($res_ty)),+
                    }

                    use $crate::{IntoSplit, TypedAsyncRead, TypedAsyncWrite};
                    let (mut writer, mut reader) = transport.into_split();
                    let reader_task = tokio::spawn(async move {
                        loop {
                            match reader.read().await {$(
                                Ok(Some([<$name:camel In>]::[<$transport:camel>](x))) => {
                                    // TODO: Handle closed channel in some way?
                                    let _ = [<$transport:snake _inbound_tx>].send(x).await;
                                }

                                // Quit if the reader no longer has data
                                // NOTE: Compiler says this is unreachable, but it is?
                                #[allow(unreachable_patterns)]
                                Ok(None) => break,

                                // Drop any received data that does not map to something
                                // NOTE: Compiler says this is unreachable, but it is?
                                #[allow(unreachable_patterns)]
                                Err(_) => continue,
                            )+}
                        }
                    });

                    let writer_task = tokio::spawn(async move {
                        loop {
                            tokio::select! {
                                $(
                                    Some(x) = [<$transport:snake _outbound_rx>].recv() => {
                                        // TODO: Handle error with send in some way?
                                        let _ = writer.write(x).await;
                                    }
                                )+
                                else => break,
                            }
                        }
                    });

                    Self {
                        reader_task,
                        writer_task,
                        $([<$transport:snake>]),+
                    }
                }

                pub fn abort(&self) {
                    self.reader_task.abort();
                    self.writer_task.abort();
                }

                pub fn is_finished(&self) -> bool {
                    self.reader_task.is_finished() && self.writer_task.is_finished()
                }
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use crate::{FramedTransport, TypedAsyncRead, TypedAsyncWrite};
    use serde::{Deserialize, Serialize};

    // NOTE: Must implement deserialize for our router,
    //       but we also need serialize to send for our test
    #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
    struct CustomData(u8, String);

    // Creates a private `TestRouter` implementation
    //
    // 1. Transport receiving `CustomData` and sending `String`
    // 2. Transport receiving `String` and sending `u8`
    // 3. Transport receiving `bool` and sending `bool`
    // 4. Transport receiving `Result<String, bool>` and sending `Option<String>`
    router!(TestRouter {
        one: CustomData => String,
        two: String => u8,
        three: bool => bool,
        should_compile: Result<String, bool> => Option<String>,
    });

    #[tokio::test]
    async fn router_should_wire_transports_to_distinguish_incoming_data() {
        let (t1, mut t2) = FramedTransport::make_test_pair();
        let TestRouter {
            mut one,
            mut two,
            mut three,
            ..
        } = TestRouter::new(t1, 100, 100);

        // Send some data of different types that these transports expect
        t2.write(false).await.unwrap();
        t2.write("hello world".to_string()).await.unwrap();
        t2.write(CustomData(123, "goodbye world".to_string()))
            .await
            .unwrap();

        // Get that data through the appropriate transport
        let data = one.read().await.unwrap().unwrap();
        assert_eq!(
            data,
            CustomData(123, "goodbye world".to_string()),
            "string_custom_data_transport got unexpected result"
        );

        let data = two.read().await.unwrap().unwrap();
        assert_eq!(
            data, "hello world",
            "u8_string_transport got unexpected result"
        );

        let data = three.read().await.unwrap().unwrap();
        assert!(!data, "bool_bool_transport got unexpected result");
    }

    #[tokio::test]
    async fn router_should_wire_transports_to_ignore_unknown_incoming_data() {
        let (t1, mut t2) = FramedTransport::make_test_pair();
        let TestRouter {
            mut one, mut two, ..
        } = TestRouter::new(t1, 100, 100);

        #[derive(Serialize, Deserialize)]
        struct UnknownData(char, u8);

        // Send some known and unknown data
        t2.write("hello world".to_string()).await.unwrap();
        t2.write(UnknownData('a', 99)).await.unwrap();
        t2.write(CustomData(123, "goodbye world".to_string()))
            .await
            .unwrap();

        // Get that data through the appropriate transport
        let data = one.read().await.unwrap().unwrap();
        assert_eq!(
            data,
            CustomData(123, "goodbye world".to_string()),
            "string_custom_data_transport got unexpected result"
        );

        let data = two.read().await.unwrap().unwrap();
        assert_eq!(
            data, "hello world",
            "u8_string_transport got unexpected result"
        );
    }

    #[tokio::test]
    async fn router_should_wire_transports_to_relay_outgoing_data() {
        let (t1, mut t2) = FramedTransport::make_test_pair();
        let TestRouter {
            mut one,
            mut two,
            mut three,
            ..
        } = TestRouter::new(t1, 100, 100);

        // NOTE: Introduce a sleep between each send, otherwise we are
        //       resolving futures in a way where the ordering may
        //       get mixed up on the way out
        async fn wait() {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        // Send some data of different types that these transports expect
        three.write(true).await.unwrap();
        wait().await;
        two.write(123).await.unwrap();
        wait().await;
        one.write("hello world".to_string()).await.unwrap();

        // All of that data should funnel through our primary transport,
        // but the order is NOT guaranteed! So we need to store
        let data: bool = t2.read().await.unwrap().unwrap();
        assert!(
            data,
            "Unexpected data received from bool_bool_transport output"
        );

        let data: u8 = t2.read().await.unwrap().unwrap();
        assert_eq!(
            data, 123,
            "Unexpected data received from u8_string_transport output"
        );

        let data: String = t2.read().await.unwrap().unwrap();
        assert_eq!(
            data, "hello world",
            "Unexpected data received from string_custom_data_transport output"
        );
    }
}
