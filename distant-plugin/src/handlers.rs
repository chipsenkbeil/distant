use std::future::Future;
use std::io;

use async_trait::async_trait;
use distant_core_auth::Authenticator;

use crate::client::Client;
use crate::common::{Destination, Map};

/// Boxed [`LaunchHandler`].
pub type BoxedLaunchHandler = Box<dyn LaunchHandler>;

/// Boxed [`ConnectHandler`].
pub type BoxedConnectHandler = Box<dyn ConnectHandler>;

/// Interface for a handler to launch a server, returning the destination to the server.
#[async_trait]
pub trait LaunchHandler: Send + Sync {
    /// Launches a server using the target `destination`. If the destination is unsupported, this
    /// method will return an error.
    ///
    /// * Takes `options` as additional parameters custom to the destination.
    /// * Takes `authenticator` to handle any authentication needs.
    async fn launch(
        &self,
        destination: &Destination,
        options: &Map,
        authenticator: &mut dyn Authenticator,
    ) -> io::Result<Destination>;
}

#[async_trait]
impl<F, R> LaunchHandler for F
where
    F: Fn(&Destination, &Map, &mut dyn Authenticator) -> R + Send + Sync + 'static,
    R: Future<Output = io::Result<Destination>> + Send + 'static,
{
    async fn launch(
        &self,
        destination: &Destination,
        options: &Map,
        authenticator: &mut dyn Authenticator,
    ) -> io::Result<Destination> {
        self(destination, options, authenticator).await
    }
}

/// Generates a new [`LaunchHandler`] for the provided anonymous function.
///
/// ### Examples
///
/// ```
/// use distant_plugin::boxed_launch_handler;
///
/// let _handler = boxed_launch_handler!(|destination, options, authenticator| {
///     todo!("Implement handler logic.");
/// });
///
/// let _handler = boxed_launch_handler!(|destination, options, authenticator| async {
///     todo!("We support async within as well regardless of the keyword!");
/// });
///
/// let _handler = boxed_launch_handler!(move |destination, options, authenticator| {
///     todo!("You can also explicitly mark to move into the closure");
/// });
/// ```
#[macro_export]
macro_rules! boxed_launch_handler {
    (|$destination:ident, $options:ident, $authenticator:ident| $(async)? $body:block) => {{
        let x: $crate::handlers::BoxedLaunchHandler = Box::new(
            |$destination: &$crate::common::Destination,
             $options: &$crate::common::Map,
             $authenticator: &mut dyn $crate::auth::Authenticator| async { $body },
        );
        x
    }};
    (move |$destination:ident, $options:ident, $authenticator:ident| $(async)? $body:block) => {{
        let x: $crate::handlers::BoxedLaunchHandler = Box::new(
            move |$destination: &$crate::common::Destination,
                  $options: &$crate::common::Map,
                  $authenticator: &mut dyn $crate::auth::Authenticator| async move { $body },
        );
        x
    }};
}

/// Interface for a handler to connect to a server, returning a boxed client to the server.
#[async_trait]
pub trait ConnectHandler: Send + Sync {
    /// Connects to a server at the specified `destination`. If the destination is unsupported,
    /// this method will return an error.
    ///
    /// * Takes `options` as additional parameters custom to the destination.
    /// * Takes `authenticator` to handle any authentication needs.
    async fn connect(
        &self,
        destination: &Destination,
        options: &Map,
        authenticator: &mut dyn Authenticator,
    ) -> io::Result<Box<dyn Client>>;
}

#[async_trait]
impl<F, R> ConnectHandler for F
where
    F: Fn(&Destination, &Map, &mut dyn Authenticator) -> R + Send + Sync + 'static,
    R: Future<Output = io::Result<Box<dyn Client>>> + Send + 'static,
{
    async fn connect(
        &self,
        destination: &Destination,
        options: &Map,
        authenticator: &mut dyn Authenticator,
    ) -> io::Result<Box<dyn Client>> {
        self(destination, options, authenticator).await
    }
}

/// Generates a new [`ConnectHandler`] for the provided anonymous function.
///
/// ### Examples
///
/// ```
/// use distant_plugin::boxed_connect_handler;
///
/// let _handler = boxed_connect_handler!(|destination, options, authenticator| {
///     todo!("Implement handler logic.");
/// });
///
/// let _handler = boxed_connect_handler!(|destination, options, authenticator| async {
///     todo!("We support async within as well regardless of the keyword!");
/// });
///
/// let _handler = boxed_connect_handler!(move |destination, options, authenticator| {
///     todo!("You can also explicitly mark to move into the closure");
/// });
/// ```
#[macro_export]
macro_rules! boxed_connect_handler {
    (|$destination:ident, $options:ident, $authenticator:ident| $(async)? $body:block) => {{
        let x: $crate::handlers::BoxedConnectHandler = Box::new(
            |$destination: &$crate::common::Destination,
             $options: &$crate::common::Map,
             $authenticator: &mut dyn $crate::auth::Authenticator| async { $body },
        );
        x
    }};
    (move |$destination:ident, $options:ident, $authenticator:ident| $(async)? $body:block) => {{
        let x: $crate::handlers::BoxedConnectHandler = Box::new(
            move |$destination: &$crate::common::Destination,
                  $options: &$crate::common::Map,
                  $authenticator: &mut dyn $crate::auth::Authenticator| async move { $body },
        );
        x
    }};
}

#[cfg(test)]
mod tests {
    use distant_core_auth::msg::*;
    use test_log::test;

    use super::*;

    #[inline]
    fn test_destination() -> Destination {
        "scheme://host:1234".parse().unwrap()
    }

    #[inline]
    fn test_options() -> Map {
        Map::default()
    }

    /// Creates an authenticator that does nothing.
    #[inline]
    fn test_authenticator() -> impl Authenticator {
        struct __TestAuthenticator;

        #[async_trait]
        impl Authenticator for __TestAuthenticator {
            async fn initialize(
                &mut self,
                _initialization: Initialization,
            ) -> io::Result<InitializationResponse> {
                unimplemented!()
            }

            async fn challenge(&mut self, _challenge: Challenge) -> io::Result<ChallengeResponse> {
                unimplemented!()
            }

            async fn verify(
                &mut self,
                _verification: Verification,
            ) -> io::Result<VerificationResponse> {
                unimplemented!()
            }

            async fn info(&mut self, _info: Info) -> io::Result<()> {
                unimplemented!()
            }

            async fn error(&mut self, _error: Error) -> io::Result<()> {
                unimplemented!()
            }

            async fn start_method(&mut self, _start_method: StartMethod) -> io::Result<()> {
                unimplemented!()
            }

            async fn finished(&mut self) -> io::Result<()> {
                unimplemented!()
            }
        }

        __TestAuthenticator
    }

    #[test(tokio::test)]
    async fn boxed_launch_handler_should_generate_valid_boxed_launch_handler() {
        let handler = boxed_launch_handler!(|_destination, _options, _authenticator| {
            Err(io::Error::from(io::ErrorKind::Other))
        });

        let Err(err) = handler
            .launch(
                &test_destination(),
                &test_options(),
                &mut test_authenticator(),
            )
            .await
        else {
            panic!("Launch succeeded unexpectedly");
        };
        assert_eq!(err.kind(), io::ErrorKind::Other);

        let handler = boxed_launch_handler!(|_destination, _options, _authenticator| async {
            Err(io::Error::from(io::ErrorKind::Other))
        });

        let Err(err) = handler
            .launch(
                &test_destination(),
                &test_options(),
                &mut test_authenticator(),
            )
            .await
        else {
            panic!("Launch succeeded unexpectedly");
        };
        assert_eq!(err.kind(), io::ErrorKind::Other);

        let handler = boxed_launch_handler!(move |_destination, _options, _authenticator| {
            Err(io::Error::from(io::ErrorKind::Other))
        });

        let Err(err) = handler
            .launch(
                &test_destination(),
                &test_options(),
                &mut test_authenticator(),
            )
            .await
        else {
            panic!("Launch succeeded unexpectedly");
        };
        assert_eq!(err.kind(), io::ErrorKind::Other);

        let handler = boxed_launch_handler!(move |_destination, _options, _authenticator| async {
            Err(io::Error::from(io::ErrorKind::Other))
        });

        let Err(err) = handler
            .launch(
                &test_destination(),
                &test_options(),
                &mut test_authenticator(),
            )
            .await
        else {
            panic!("Launch succeeded unexpectedly");
        };
        assert_eq!(err.kind(), io::ErrorKind::Other);
    }

    #[test(tokio::test)]
    async fn boxed_connect_handler_should_generate_valid_boxed_connect_handler() {
        let handler = boxed_connect_handler!(|_destination, _options, _authenticator| {
            Err(io::Error::from(io::ErrorKind::Other))
        });

        let Err(err) = handler
            .connect(
                &test_destination(),
                &test_options(),
                &mut test_authenticator(),
            )
            .await
        else {
            panic!("Connect succeeded unexpectedly");
        };
        assert_eq!(err.kind(), io::ErrorKind::Other);

        let handler = boxed_connect_handler!(|_destination, _options, _authenticator| async {
            Err(io::Error::from(io::ErrorKind::Other))
        });

        let Err(err) = handler
            .connect(
                &test_destination(),
                &test_options(),
                &mut test_authenticator(),
            )
            .await
        else {
            panic!("Connect succeeded unexpectedly");
        };
        assert_eq!(err.kind(), io::ErrorKind::Other);

        let handler = boxed_connect_handler!(move |_destination, _options, _authenticator| {
            Err(io::Error::from(io::ErrorKind::Other))
        });

        let Err(err) = handler
            .connect(
                &test_destination(),
                &test_options(),
                &mut test_authenticator(),
            )
            .await
        else {
            panic!("Connect succeeded unexpectedly");
        };
        assert_eq!(err.kind(), io::ErrorKind::Other);

        let handler = boxed_connect_handler!(move |_destination, _options, _authenticator| async {
            Err(io::Error::from(io::ErrorKind::Other))
        });

        let Err(err) = handler
            .connect(
                &test_destination(),
                &test_options(),
                &mut test_authenticator(),
            )
            .await
        else {
            panic!("Connect succeeded unexpectedly");
        };
        assert_eq!(err.kind(), io::ErrorKind::Other);
    }
}
