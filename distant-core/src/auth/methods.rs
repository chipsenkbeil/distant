use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::str::FromStr;

use log::*;

use crate::auth::authenticator::Authenticator;
use crate::auth::msg::*;

mod none;
mod static_key;

pub use none::*;
pub use static_key::*;

/// Supports authenticating using a variety of methods
pub struct Verifier {
    methods: HashMap<&'static str, Box<dyn AuthenticationMethod>>,
}

impl Verifier {
    pub fn new<I>(methods: I) -> Self
    where
        I: IntoIterator<Item = Box<dyn AuthenticationMethod>>,
    {
        let mut m = HashMap::new();

        for method in methods {
            m.insert(method.id(), method);
        }

        Self { methods: m }
    }

    /// Creates a verifier with no methods.
    pub fn empty() -> Self {
        Self {
            methods: HashMap::new(),
        }
    }

    /// Creates a verifier that uses the [`NoneAuthenticationMethod`] exclusively.
    pub fn none() -> Self {
        Self::new(vec![
            Box::new(NoneAuthenticationMethod::new()) as Box<dyn AuthenticationMethod>
        ])
    }

    /// Creates a verifier that uses the [`StaticKeyAuthenticationMethod`] exclusively.
    pub fn static_key<K>(key: K) -> Self
    where
        K: FromStr + PartialEq + Send + Sync + 'static,
    {
        Self::new(vec![
            Box::new(StaticKeyAuthenticationMethod::new(key)) as Box<dyn AuthenticationMethod>
        ])
    }

    /// Returns an iterator over the ids of the methods supported by the verifier
    pub fn methods(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.methods.keys().copied()
    }

    /// Attempts to verify by submitting challenges using the `authenticator` provided. Returns the
    /// id of the authentication method that succeeded. Fails if no authentication method succeeds.
    pub async fn verify(&self, authenticator: &mut dyn Authenticator) -> io::Result<&'static str> {
        // Initiate the process to get methods to use
        let response = authenticator
            .initialize(Initialization {
                methods: self.methods.keys().map(ToString::to_string).collect(),
            })
            .await?;

        for method in response.methods {
            match self.methods.get(method.as_str()) {
                Some(method) => {
                    // Report the authentication method
                    authenticator
                        .start_method(StartMethod {
                            method: method.id().to_string(),
                        })
                        .await?;

                    // Perform the actual authentication
                    if method.authenticate(authenticator).await.is_ok() {
                        authenticator.finished().await?;
                        return Ok(method.id());
                    }
                }
                None => {
                    trace!("Skipping authentication {method} as it is not available or supported");
                }
            }
        }

        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "No authentication method succeeded",
        ))
    }
}

impl From<Vec<Box<dyn AuthenticationMethod>>> for Verifier {
    fn from(methods: Vec<Box<dyn AuthenticationMethod>>) -> Self {
        Self::new(methods)
    }
}

/// Represents an interface to authenticate using some method
pub trait AuthenticationMethod: Send + Sync {
    /// Returns a unique id to distinguish the method from other methods
    fn id(&self) -> &'static str;

    /// Performs authentication using the `authenticator` to submit challenges and other
    /// information based on the authentication method
    fn authenticate<'a>(
        &'a self,
        authenticator: &'a mut dyn Authenticator,
    ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>>;
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::mpsc;

    use test_log::test;

    use super::*;
    use crate::auth::authenticator::TestAuthenticator;

    struct SuccessAuthenticationMethod;

    impl AuthenticationMethod for SuccessAuthenticationMethod {
        fn id(&self) -> &'static str {
            "success"
        }

        fn authenticate<'a>(
            &'a self,
            _: &'a mut dyn Authenticator,
        ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
            Box::pin(async move { Ok(()) })
        }
    }

    struct FailAuthenticationMethod;

    impl AuthenticationMethod for FailAuthenticationMethod {
        fn id(&self) -> &'static str {
            "fail"
        }

        fn authenticate<'a>(
            &'a self,
            _: &'a mut dyn Authenticator,
        ) -> Pin<Box<dyn Future<Output = io::Result<()>> + Send + 'a>> {
            Box::pin(async move { Err(io::Error::from(io::ErrorKind::Other)) })
        }
    }

    #[test(tokio::test)]
    async fn verifier_should_fail_to_verify_if_initialization_fails() {
        let mut authenticator = TestAuthenticator {
            initialize: Box::new(|_| Err(io::Error::from(io::ErrorKind::Other))),
            ..Default::default()
        };

        let methods: Vec<Box<dyn AuthenticationMethod>> =
            vec![Box::new(SuccessAuthenticationMethod)];
        let verifier = Verifier::from(methods);
        verifier.verify(&mut authenticator).await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn verifier_should_fail_to_verify_if_fails_to_send_finished_indicator_after_success() {
        let mut authenticator = TestAuthenticator {
            initialize: Box::new(|_| {
                Ok(InitializationResponse {
                    methods: vec![SuccessAuthenticationMethod.id().to_string()]
                        .into_iter()
                        .collect(),
                })
            }),
            finished: Box::new(|| Err(io::Error::other("test error"))),
            ..Default::default()
        };

        let methods: Vec<Box<dyn AuthenticationMethod>> =
            vec![Box::new(SuccessAuthenticationMethod)];
        let verifier = Verifier::from(methods);

        let err = verifier.verify(&mut authenticator).await.unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);
        assert_eq!(err.to_string(), "test error");
    }

    #[test(tokio::test)]
    async fn verifier_should_fail_to_verify_if_has_no_authentication_methods() {
        let mut authenticator = TestAuthenticator {
            initialize: Box::new(|_| {
                Ok(InitializationResponse {
                    methods: vec![SuccessAuthenticationMethod.id().to_string()]
                        .into_iter()
                        .collect(),
                })
            }),
            ..Default::default()
        };

        let methods: Vec<Box<dyn AuthenticationMethod>> = vec![];
        let verifier = Verifier::from(methods);
        verifier.verify(&mut authenticator).await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn verifier_should_fail_to_verify_if_initialization_yields_no_valid_authentication_methods()
     {
        let mut authenticator = TestAuthenticator {
            initialize: Box::new(|_| {
                Ok(InitializationResponse {
                    methods: vec!["other".to_string()].into_iter().collect(),
                })
            }),
            ..Default::default()
        };

        let methods: Vec<Box<dyn AuthenticationMethod>> =
            vec![Box::new(SuccessAuthenticationMethod)];
        let verifier = Verifier::from(methods);
        verifier.verify(&mut authenticator).await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn verifier_should_fail_to_verify_if_no_authentication_method_succeeds() {
        let mut authenticator = TestAuthenticator {
            initialize: Box::new(|_| {
                Ok(InitializationResponse {
                    methods: vec![FailAuthenticationMethod.id().to_string()]
                        .into_iter()
                        .collect(),
                })
            }),
            ..Default::default()
        };

        let methods: Vec<Box<dyn AuthenticationMethod>> = vec![Box::new(FailAuthenticationMethod)];
        let verifier = Verifier::from(methods);
        verifier.verify(&mut authenticator).await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn verifier_should_return_id_of_authentication_method_upon_success() {
        let mut authenticator = TestAuthenticator {
            initialize: Box::new(|_| {
                Ok(InitializationResponse {
                    methods: vec![SuccessAuthenticationMethod.id().to_string()]
                        .into_iter()
                        .collect(),
                })
            }),
            ..Default::default()
        };

        let methods: Vec<Box<dyn AuthenticationMethod>> =
            vec![Box::new(SuccessAuthenticationMethod)];
        let verifier = Verifier::from(methods);
        assert_eq!(
            verifier.verify(&mut authenticator).await.unwrap(),
            SuccessAuthenticationMethod.id()
        );
    }

    #[test(tokio::test)]
    async fn verifier_should_try_authentication_methods_in_order_until_one_succeeds() {
        let mut authenticator = TestAuthenticator {
            initialize: Box::new(|_| {
                Ok(InitializationResponse {
                    methods: vec![
                        FailAuthenticationMethod.id().to_string(),
                        SuccessAuthenticationMethod.id().to_string(),
                    ]
                    .into_iter()
                    .collect(),
                })
            }),
            ..Default::default()
        };

        let methods: Vec<Box<dyn AuthenticationMethod>> = vec![
            Box::new(FailAuthenticationMethod),
            Box::new(SuccessAuthenticationMethod),
        ];
        let verifier = Verifier::from(methods);
        assert_eq!(
            verifier.verify(&mut authenticator).await.unwrap(),
            SuccessAuthenticationMethod.id()
        );
    }

    #[test(tokio::test)]
    async fn verifier_should_send_start_method_before_attempting_each_method() {
        let (tx, rx) = mpsc::channel();

        let mut authenticator = TestAuthenticator {
            initialize: Box::new(|_| {
                Ok(InitializationResponse {
                    methods: vec![
                        FailAuthenticationMethod.id().to_string(),
                        SuccessAuthenticationMethod.id().to_string(),
                    ]
                    .into_iter()
                    .collect(),
                })
            }),
            start_method: Box::new(move |method| {
                tx.send(method.method).unwrap();
                Ok(())
            }),
            ..Default::default()
        };

        let methods: Vec<Box<dyn AuthenticationMethod>> = vec![
            Box::new(FailAuthenticationMethod),
            Box::new(SuccessAuthenticationMethod),
        ];
        Verifier::from(methods)
            .verify(&mut authenticator)
            .await
            .unwrap();

        assert_eq!(rx.try_recv().unwrap(), FailAuthenticationMethod.id());
        assert_eq!(rx.try_recv().unwrap(), SuccessAuthenticationMethod.id());
        assert_eq!(rx.try_recv().unwrap_err(), mpsc::TryRecvError::Empty);
    }

    #[test(tokio::test)]
    async fn verifier_should_send_finished_when_a_method_succeeds() {
        let (tx, rx) = mpsc::channel();

        let mut authenticator = TestAuthenticator {
            initialize: Box::new(|_| {
                Ok(InitializationResponse {
                    methods: vec![
                        FailAuthenticationMethod.id().to_string(),
                        SuccessAuthenticationMethod.id().to_string(),
                    ]
                    .into_iter()
                    .collect(),
                })
            }),
            finished: Box::new(move || {
                tx.send(()).unwrap();
                Ok(())
            }),
            ..Default::default()
        };

        let methods: Vec<Box<dyn AuthenticationMethod>> = vec![
            Box::new(FailAuthenticationMethod),
            Box::new(SuccessAuthenticationMethod),
        ];
        Verifier::from(methods)
            .verify(&mut authenticator)
            .await
            .unwrap();

        rx.try_recv().unwrap();
        assert_eq!(rx.try_recv().unwrap_err(), mpsc::TryRecvError::Empty);
    }
}
