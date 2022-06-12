use paste::paste;

/// Creates a new struct around a [`FramedTransport`] that
/// routes incoming messages
macro_rules! router {
    ($name:ident: $($req:ident -> $res:ident),*) => {
        paste! {
            pub struct $name<C>
            where
                C: $crate::Codec
            {
                task: tokio::task::JoinHandle<()>,
                $(
                    pub [<$req:snake _ $res:snake _ transport>]: $crate::FramedTransport<
                        $crate::InmemoryTransport,
                        C
                    >
                ),+
            }

            impl<C> $name<C>
                where C: $crate::Codec
            {
                pub fn new<R, W>(mut reader: R, mut writer: W) -> Self
                where
                    R: $crate::FramedTransportRead + Send + 'static,
                    W: $crate::FramedTransportWrite + Send + 'static,
                {
                    #[derive(serde::Deserialize)]
                    #[serde(untagged)]
                    enum [<$name:camel Data>] {
                        $([<$res:camel>]($res)),+
                    }

                    $(
                        let [<$req:snake _ $res:snake _ >]
                    )+

                    // Spawn a general-purpose
                    let task = tokio::spawn(async move {
                    });

                    Self {
                        task,
                        $(
                            [<$req:snake _ $res:snake _ transport>]: 0,
                        )+
                    }
                }
            }
        }
    };
}

router!(TestRouter: u16 -> u32, bool -> String);
