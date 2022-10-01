use super::{AuthenticationMethod, Authenticator, Challenge, Error, Question};
use crate::HeapSecretKey;
use async_trait::async_trait;
use std::io;

/// Authenticaton method for a static secret key
#[derive(Clone, Debug)]
pub struct StaticKeyAuthenticationMethod {
    key: HeapSecretKey,
}

impl StaticKeyAuthenticationMethod {
    #[inline]
    pub fn new(key: impl Into<HeapSecretKey>) -> Self {
        Self { key: key.into() }
    }
}

#[async_trait]
impl AuthenticationMethod for StaticKeyAuthenticationMethod {
    fn id(&self) -> &'static str {
        "static_key"
    }

    async fn authenticate(&self, authenticator: &mut dyn Authenticator) -> io::Result<()> {
        let response = authenticator
            .challenge(Challenge {
                questions: vec![Question::new("key")],
                options: Default::default(),
            })
            .await?;

        if response.answers.is_empty() {
            let x = Error::fatal("missing answer");
            authenticator.error(x.clone()).await?;
            return Err(x.into_io_permission_denied());
        } else if response.answers.len() > 1 {
            authenticator
                .error(Error::non_fatal("more than one answer, picking first"))
                .await?;
        }

        match response
            .answers
            .into_iter()
            .next()
            .unwrap()
            .parse::<HeapSecretKey>()
        {
            Ok(key) if key == self.key => Ok(()),
            _ => {
                let x = Error::fatal("answer not a valid key");
                authenticator.error(x.clone()).await?;
                Err(x.into_io_permission_denied())
            }
        }
    }
}
