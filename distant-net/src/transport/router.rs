/// Creates a new struct around a [`FramedTransport`] that routes incoming and outgoing messages
///
/// ```
///
/// ```
#[macro_export]
macro_rules! router {
    ($name:ident: $($req:ident -> $res:ident),+) => {
        $crate::paste::paste! {
            pub struct $name {
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
                        while let Ok(Some(data)) = reader.recv().await {
                            match data {$(
                                [<$name:camel In>]::[<$res:camel>](x) => {
                                    // TODO: Handle closed channel in some way?
                                    let _ = [<$req:snake _ $res:snake _ transport_inbound_tx>].send(x).await;
                                }
                            )+}
                        }
                    });

                    let writer_task = tokio::spawn(async move {
                        loop {
                            tokio::select! {
                                $(
                                    Some(x) = [<$req:snake _ $res:snake _ transport_outbound_rx>].recv() => {
                                        // TODO: Handle error with send in some way?
                                        let _ = writer.send(x).await;
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

router!(TestRouter: u16 -> u32, bool -> String);
