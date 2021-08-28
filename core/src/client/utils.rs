use std::{future::Future, time::Duration};
use tokio::{io, time};

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
