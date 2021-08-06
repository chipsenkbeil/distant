use std::{
    future::Future,
    ops::{Deref, DerefMut},
    time::Duration,
};
use tokio::{io, time};

// Generates a new tenant name
pub fn new_tenant() -> String {
    format!("tenant_{}{}", rand::random::<u16>(), rand::random::<u8>())
}

// Wraps a future in a tokio timeout call, transforming the error into
// an io error
pub async fn timeout<T, F>(d: Duration, f: F) -> io::Result<T>
where
    F: Future<Output = T>,
{
    time::timeout(d, f)
        .await
        .map_err(|x| io::Error::new(io::ErrorKind::TimedOut, x))
}

/// Wraps a string to provide some friendly read and write methods
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StringBuf(String);

impl StringBuf {
    pub fn new() -> Self {
        Self(String::new())
    }

    /// Consumes data within the buffer that represent full lines (end with a newline) and returns
    /// the string containing those lines.
    ///
    /// The remaining buffer contains are returned as the second part of the tuple
    pub fn into_full_lines(mut self) -> (Option<String>, StringBuf) {
        match self.rfind('\n') {
            Some(idx) => {
                let remaining = self.0.split_off(idx + 1);
                (Some(self.0), Self(remaining))
            }
            None => (None, self),
        }
    }
}

impl From<String> for StringBuf {
    fn from(x: String) -> Self {
        Self(x)
    }
}

impl From<StringBuf> for String {
    fn from(x: StringBuf) -> Self {
        x.0
    }
}

impl Deref for StringBuf {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for StringBuf {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
