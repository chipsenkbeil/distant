use super::{super::HeapSecretKey, msg::*, Authenticator};
use async_trait::async_trait;
use log::*;
use std::collections::HashMap;
use std::io;

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
    pub fn static_key(key: impl Into<HeapSecretKey>) -> Self {
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
#[async_trait]
pub trait AuthenticationMethod: Send + Sync {
    /// Returns a unique id to distinguish the method from other methods
    fn id(&self) -> &'static str;

    /// Performs authentication using the `authenticator` to submit challenges and other
    /// information based on the authentication method
    async fn authenticate(&self, authenticator: &mut dyn Authenticator) -> io::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::FramedTransport;
    use test_log::test;

    struct SuccessAuthenticationMethod;

    #[async_trait]
    impl AuthenticationMethod for SuccessAuthenticationMethod {
        fn id(&self) -> &'static str {
            "success"
        }

        async fn authenticate(&self, _: &mut dyn Authenticator) -> io::Result<()> {
            Ok(())
        }
    }

    struct FailAuthenticationMethod;

    #[async_trait]
    impl AuthenticationMethod for FailAuthenticationMethod {
        fn id(&self) -> &'static str {
            "fail"
        }

        async fn authenticate(&self, _: &mut dyn Authenticator) -> io::Result<()> {
            Err(io::Error::from(io::ErrorKind::Other))
        }
    }

    #[test(tokio::test)]
    async fn verifier_should_fail_to_verify_if_initialization_fails() {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // Queue up a response to the initialization request
        t2.write_frame(b"invalid initialization response")
            .await
            .unwrap();

        let methods: Vec<Box<dyn AuthenticationMethod>> =
            vec![Box::new(SuccessAuthenticationMethod)];
        let verifier = Verifier::from(methods);
        verifier.verify(&mut t1).await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn verifier_should_fail_to_verify_if_fails_to_send_finished_indicator_after_success() {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // Queue up a response to the initialization request
        t2.write_frame_for(&AuthenticationResponse::Initialization(
            InitializationResponse {
                methods: vec![SuccessAuthenticationMethod.id().to_string()]
                    .into_iter()
                    .collect(),
            },
        ))
        .await
        .unwrap();

        // Then drop the transport so it cannot receive anything else
        drop(t2);

        let methods: Vec<Box<dyn AuthenticationMethod>> =
            vec![Box::new(SuccessAuthenticationMethod)];
        let verifier = Verifier::from(methods);
        assert_eq!(
            verifier.verify(&mut t1).await.unwrap_err().kind(),
            io::ErrorKind::WriteZero
        );
    }

    #[test(tokio::test)]
    async fn verifier_should_fail_to_verify_if_has_no_authentication_methods() {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // Queue up a response to the initialization request
        t2.write_frame_for(&AuthenticationResponse::Initialization(
            InitializationResponse {
                methods: vec![SuccessAuthenticationMethod.id().to_string()]
                    .into_iter()
                    .collect(),
            },
        ))
        .await
        .unwrap();

        let methods: Vec<Box<dyn AuthenticationMethod>> = vec![];
        let verifier = Verifier::from(methods);
        verifier.verify(&mut t1).await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn verifier_should_fail_to_verify_if_initialization_yields_no_valid_authentication_methods(
    ) {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // Queue up a response to the initialization request
        t2.write_frame_for(&AuthenticationResponse::Initialization(
            InitializationResponse {
                methods: vec!["other".to_string()].into_iter().collect(),
            },
        ))
        .await
        .unwrap();

        let methods: Vec<Box<dyn AuthenticationMethod>> =
            vec![Box::new(SuccessAuthenticationMethod)];
        let verifier = Verifier::from(methods);
        verifier.verify(&mut t1).await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn verifier_should_fail_to_verify_if_no_authentication_method_succeeds() {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // Queue up a response to the initialization request
        t2.write_frame_for(&AuthenticationResponse::Initialization(
            InitializationResponse {
                methods: vec![FailAuthenticationMethod.id().to_string()]
                    .into_iter()
                    .collect(),
            },
        ))
        .await
        .unwrap();

        let methods: Vec<Box<dyn AuthenticationMethod>> = vec![Box::new(FailAuthenticationMethod)];
        let verifier = Verifier::from(methods);
        verifier.verify(&mut t1).await.unwrap_err();
    }

    #[test(tokio::test)]
    async fn verifier_should_return_id_of_authentication_method_upon_success() {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // Queue up a response to the initialization request
        t2.write_frame_for(&AuthenticationResponse::Initialization(
            InitializationResponse {
                methods: vec![SuccessAuthenticationMethod.id().to_string()]
                    .into_iter()
                    .collect(),
            },
        ))
        .await
        .unwrap();

        let methods: Vec<Box<dyn AuthenticationMethod>> =
            vec![Box::new(SuccessAuthenticationMethod)];
        let verifier = Verifier::from(methods);
        assert_eq!(
            verifier.verify(&mut t1).await.unwrap(),
            SuccessAuthenticationMethod.id()
        );
    }

    #[test(tokio::test)]
    async fn verifier_should_try_authentication_methods_in_order_until_one_succeeds() {
        let (mut t1, mut t2) = FramedTransport::test_pair(100);

        // Queue up a response to the initialization request
        t2.write_frame_for(&AuthenticationResponse::Initialization(
            InitializationResponse {
                methods: vec![
                    FailAuthenticationMethod.id().to_string(),
                    SuccessAuthenticationMethod.id().to_string(),
                ]
                .into_iter()
                .collect(),
            },
        ))
        .await
        .unwrap();

        let methods: Vec<Box<dyn AuthenticationMethod>> = vec![
            Box::new(FailAuthenticationMethod),
            Box::new(SuccessAuthenticationMethod),
        ];
        let verifier = Verifier::from(methods);
        assert_eq!(
            verifier.verify(&mut t1).await.unwrap(),
            SuccessAuthenticationMethod.id()
        );
    }
}
