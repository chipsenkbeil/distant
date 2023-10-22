use std::io;

use async_trait::async_trait;

use super::Listener;

/// Represents a [`Listener`] that wraps a different [`Listener`],
/// mapping the received connection to something else using the map function
pub struct MappedListener<L, F, T, U>
where
    L: Listener<Output = T>,
    F: FnMut(T) -> U + Send + Sync,
{
    listener: L,
    f: F,
}

impl<L, F, T, U> MappedListener<L, F, T, U>
where
    L: Listener<Output = T>,
    F: FnMut(T) -> U + Send + Sync,
{
    pub fn new(listener: L, f: F) -> Self {
        Self { listener, f }
    }
}

#[async_trait]
impl<L, F, T, U> Listener for MappedListener<L, F, T, U>
where
    L: Listener<Output = T>,
    F: FnMut(T) -> U + Send + Sync,
{
    type Output = U;

    /// Waits for the next fully-initialized transport for an incoming stream to be available,
    /// returning an error if no longer accepting new connections
    async fn accept(&mut self) -> io::Result<Self::Output> {
        let output = self.listener.accept().await?;
        Ok((self.f)(output))
    }
}
