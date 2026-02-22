use std::future::Future;
use std::io;
use std::pin::Pin;

use crate::auth::Authenticator;

use crate::net::client::UntypedClient;
use crate::net::common::{Destination, Map};

pub type BoxedLaunchHandler = Box<dyn LaunchHandler>;
pub type BoxedConnectHandler = Box<dyn ConnectHandler>;

/// Represents an interface to start a server at some remote `destination`.
///
/// * `destination` is the location where the server will be started.
/// * `options` is provided to include extra information needed to launch or establish the
///   connection.
/// * `authenticator` is provided to support a challenge-based authentication while launching.
///
/// Returns a [`Destination`] representing the new origin to use if a connection is desired.
pub trait LaunchHandler: Send + Sync {
    fn launch<'a>(
        &'a self,
        destination: &'a Destination,
        options: &'a Map,
        authenticator: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<Destination>> + Send + 'a>>;
}

impl<F, R> LaunchHandler for F
where
    F: Fn(&Destination, &Map, &mut dyn Authenticator) -> R + Send + Sync + 'static,
    R: Future<Output = io::Result<Destination>> + Send + 'static,
{
    fn launch<'a>(
        &'a self,
        destination: &'a Destination,
        options: &'a Map,
        authenticator: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<Destination>> + Send + 'a>> {
        Box::pin(self(destination, options, authenticator))
    }
}

/// Generates a new [`LaunchHandler`] for the provided anonymous function in the form of
///
/// ```
/// use distant_core::boxed_launch_handler;
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
        let x: $crate::net::manager::BoxedLaunchHandler = Box::new(
            |$destination: &$crate::net::common::Destination,
             $options: &$crate::net::common::Map,
             $authenticator: &mut dyn $crate::auth::Authenticator| async { $body },
        );
        x
    }};
    (move |$destination:ident, $options:ident, $authenticator:ident| $(async)? $body:block) => {{
        let x: $crate::net::manager::BoxedLaunchHandler = Box::new(
            move |$destination: &$crate::net::common::Destination,
                  $options: &$crate::net::common::Map,
                  $authenticator: &mut dyn $crate::auth::Authenticator| async move { $body },
        );
        x
    }};
}

/// Represents an interface to perform a connection to some remote `destination`.
///
/// * `destination` is the location of the server to connect to.
/// * `options` is provided to include extra information needed to establish the connection.
/// * `authenticator` is provided to support a challenge-based authentication while connecting.
///
/// Returns an [`UntypedClient`] representing the connection.
pub trait ConnectHandler: Send + Sync {
    fn connect<'a>(
        &'a self,
        destination: &'a Destination,
        options: &'a Map,
        authenticator: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<UntypedClient>> + Send + 'a>>;
}

impl<F, R> ConnectHandler for F
where
    F: Fn(&Destination, &Map, &mut dyn Authenticator) -> R + Send + Sync + 'static,
    R: Future<Output = io::Result<UntypedClient>> + Send + 'static,
{
    fn connect<'a>(
        &'a self,
        destination: &'a Destination,
        options: &'a Map,
        authenticator: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<UntypedClient>> + Send + 'a>> {
        Box::pin(self(destination, options, authenticator))
    }
}

/// Generates a new [`ConnectHandler`] for the provided anonymous function in the form of
///
/// ```
/// use distant_core::boxed_connect_handler;
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
        let x: $crate::net::manager::BoxedConnectHandler = Box::new(
            |$destination: &$crate::net::common::Destination,
             $options: &$crate::net::common::Map,
             $authenticator: &mut dyn $crate::auth::Authenticator| async { $body },
        );
        x
    }};
    (move |$destination:ident, $options:ident, $authenticator:ident| $(async)? $body:block) => {{
        let x: $crate::net::manager::BoxedConnectHandler = Box::new(
            move |$destination: &$crate::net::common::Destination,
                  $options: &$crate::net::common::Map,
                  $authenticator: &mut dyn $crate::auth::Authenticator| async move { $body },
        );
        x
    }};
}

#[cfg(test)]
mod tests {
    use test_log::test;

    use super::*;
    use crate::net::common::FramedTransport;

    #[inline]
    fn test_destination() -> Destination {
        "scheme://host:1234".parse().unwrap()
    }

    #[inline]
    fn test_options() -> Map {
        Map::default()
    }

    #[inline]
    fn test_authenticator() -> impl Authenticator {
        FramedTransport::pair(1).0
    }

    #[test(tokio::test)]
    async fn boxed_launch_handler_should_generate_valid_boxed_launch_handler() {
        let handler = boxed_launch_handler!(|_destination, _options, _authenticator| {
            Err(io::Error::from(io::ErrorKind::Other))
        });
        assert_eq!(
            handler
                .launch(
                    &test_destination(),
                    &test_options(),
                    &mut test_authenticator()
                )
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::Other
        );

        let handler = boxed_launch_handler!(|_destination, _options, _authenticator| async {
            Err(io::Error::from(io::ErrorKind::Other))
        });
        assert_eq!(
            handler
                .launch(
                    &test_destination(),
                    &test_options(),
                    &mut test_authenticator()
                )
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::Other
        );

        let handler = boxed_launch_handler!(move |_destination, _options, _authenticator| {
            Err(io::Error::from(io::ErrorKind::Other))
        });
        assert_eq!(
            handler
                .launch(
                    &test_destination(),
                    &test_options(),
                    &mut test_authenticator()
                )
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::Other
        );

        let handler = boxed_launch_handler!(move |_destination, _options, _authenticator| async {
            Err(io::Error::from(io::ErrorKind::Other))
        });
        assert_eq!(
            handler
                .launch(
                    &test_destination(),
                    &test_options(),
                    &mut test_authenticator()
                )
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::Other
        );
    }

    #[test(tokio::test)]
    async fn boxed_connect_handler_should_generate_valid_boxed_connect_handler() {
        let handler = boxed_connect_handler!(|_destination, _options, _authenticator| {
            Err(io::Error::from(io::ErrorKind::Other))
        });
        assert_eq!(
            handler
                .connect(
                    &test_destination(),
                    &test_options(),
                    &mut test_authenticator()
                )
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::Other
        );

        let handler = boxed_connect_handler!(|_destination, _options, _authenticator| async {
            Err(io::Error::from(io::ErrorKind::Other))
        });
        assert_eq!(
            handler
                .connect(
                    &test_destination(),
                    &test_options(),
                    &mut test_authenticator()
                )
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::Other
        );

        let handler = boxed_connect_handler!(move |_destination, _options, _authenticator| {
            Err(io::Error::from(io::ErrorKind::Other))
        });
        assert_eq!(
            handler
                .connect(
                    &test_destination(),
                    &test_options(),
                    &mut test_authenticator()
                )
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::Other
        );

        let handler = boxed_connect_handler!(move |_destination, _options, _authenticator| async {
            Err(io::Error::from(io::ErrorKind::Other))
        });
        assert_eq!(
            handler
                .connect(
                    &test_destination(),
                    &test_options(),
                    &mut test_authenticator()
                )
                .await
                .unwrap_err()
                .kind(),
            io::ErrorKind::Other
        );
    }
}
