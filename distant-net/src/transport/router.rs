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
/// router! {
///     TestRouter:
///         u8 -> String,
///         bool -> CustomData,
/// }
///
/// # let (transport, _) = distant_net::FramedTransport::pair(1);
///
/// let router = TestRouter::new(
///     /* FramedTransport */ transport,
///     /* inbound_buffer  */ 100,
///     /* outbound_buffer */ 100,
/// );
/// ```
#[macro_export]
macro_rules! router {
    ($vis:vis $name:ident: $($req:ident -> $res:ident),+ $(,)?) => {
        $crate::paste::paste! {
            #[allow(dead_code)]
            $vis struct $name {
                reader_task: tokio::task::JoinHandle<()>,
                writer_task: tokio::task::JoinHandle<()>,
                $(
                    pub [<$req:snake _ $res:snake _ transport>]:
                        $crate::MpscTransport<$req, $res>,
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
                            [<$req:snake _ $res:snake _ transport_inbound_tx>],
                            [<$req:snake _ $res:snake _ transport_inbound_rx>]
                        ) = tokio::sync::mpsc::channel(inbound_buffer);
                        let (
                            [<$req:snake _ $res:snake _ transport_outbound_tx>],
                            mut [<$req:snake _ $res:snake _ transport_outbound_rx>]
                        ) = tokio::sync::mpsc::channel(outbound_buffer);
                        let [<$req:snake _ $res:snake _ transport>]:
                            $crate::MpscTransport<$req, $res> = $crate::MpscTransport::new(
                            [<$req:snake _ $res:snake _ transport_outbound_tx>],
                            [<$req:snake _ $res:snake _ transport_inbound_rx>]
                        );
                    )+

                    #[derive(serde::Deserialize)]
                    #[serde(untagged)]
                    enum [<$name:camel In>] {
                        $([<$res:camel>]($res)),+
                    }

                    use $crate::{IntoSplit, TypedAsyncRead, TypedAsyncWrite};
                    let (mut reader, mut writer) = transport.into_split();
                    let reader_task = tokio::spawn(async move {
                        loop {
                            match reader.read().await {$(
                                Ok(Some([<$name:camel In>]::[<$res:camel>](x))) => {
                                    // TODO: Handle closed channel in some way?
                                    let _ = [<$req:snake _ $res:snake _ transport_inbound_tx>].send(x).await;
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
                                    Some(x) = [<$req:snake _ $res:snake _ transport_outbound_rx>].recv() => {
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
                        $([<$req:snake _ $res:snake _ transport>],)+
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
    // 1. Transport sending `String` and receiving `CustomData`
    // 2. Transport sending `u8` and receiving `String`
    // 3. Transport sending `bool` and receiving `bool`
    router! {
        TestRouter:
            String -> CustomData,
            u8 -> String,
            bool -> bool,
    }

    #[tokio::test]
    async fn router_should_wire_transports_to_distinguish_incoming_data() {
        let (t1, mut t2) = FramedTransport::make_test_pair();
        let TestRouter {
            mut string_custom_data_transport,
            mut u8_string_transport,
            mut bool_bool_transport,
            ..
        } = TestRouter::new(t1, 100, 100);

        // Send some data of different types that these transports expect
        t2.write(false).await.unwrap();
        t2.write("hello world".to_string()).await.unwrap();
        t2.write(CustomData(123, "goodbye world".to_string()))
            .await
            .unwrap();

        // Get that data through the appropriate transport
        let data = string_custom_data_transport.read().await.unwrap().unwrap();
        assert_eq!(
            data,
            CustomData(123, "goodbye world".to_string()),
            "string_custom_data_transport got unexpected result"
        );

        let data = u8_string_transport.read().await.unwrap().unwrap();
        assert_eq!(
            data, "hello world",
            "u8_string_transport got unexpected result"
        );

        let data = bool_bool_transport.read().await.unwrap().unwrap();
        assert!(!data, "bool_bool_transport got unexpected result");
    }

    #[tokio::test]
    async fn router_should_wire_transports_to_ignore_unknown_incoming_data() {
        let (t1, mut t2) = FramedTransport::make_test_pair();
        let TestRouter {
            mut string_custom_data_transport,
            mut u8_string_transport,
            ..
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
        let data = string_custom_data_transport.read().await.unwrap().unwrap();
        assert_eq!(
            data,
            CustomData(123, "goodbye world".to_string()),
            "string_custom_data_transport got unexpected result"
        );

        let data = u8_string_transport.read().await.unwrap().unwrap();
        assert_eq!(
            data, "hello world",
            "u8_string_transport got unexpected result"
        );
    }

    #[tokio::test]
    async fn router_should_wire_transports_to_relay_outgoing_data() {
        let (t1, mut t2) = FramedTransport::make_test_pair();
        let TestRouter {
            mut string_custom_data_transport,
            mut u8_string_transport,
            mut bool_bool_transport,
            ..
        } = TestRouter::new(t1, 100, 100);

        // NOTE: Introduce a sleep between each send, otherwise we are
        //       resolving futures in a way where the ordering may
        //       get mixed up on the way out
        async fn wait() {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }

        // Send some data of different types that these transports expect
        bool_bool_transport.write(true).await.unwrap();
        wait().await;
        u8_string_transport.write(123).await.unwrap();
        wait().await;
        string_custom_data_transport
            .write("hello world".to_string())
            .await
            .unwrap();

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
