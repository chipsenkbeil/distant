use std::{future::Future, time::Duration};
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
