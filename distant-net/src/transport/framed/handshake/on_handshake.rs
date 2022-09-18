use super::FramedTransport;
use std::{fmt, future::Future, io, pin::Pin};

/// Boxed function representing `on_handshake` callback
pub type BoxedOnHandshakeFn<T, const CAPACITY: usize> = Box<
    dyn FnMut(&mut FramedTransport<T, CAPACITY>) -> Pin<Box<dyn Future<Output = io::Result<()>>>>,
>;

/// Callback invoked when a handshake occurs
pub struct OnHandshake<T, const CAPACITY: usize>(pub(super) BoxedOnHandshakeFn<T, CAPACITY>);

impl<T, const CAPACITY: usize> OnHandshake<T, CAPACITY> {
    /// Wraps a function `f` as a callback for a handshake
    pub fn new<F>(f: F) -> Self
    where
        F: FnMut(
            &mut FramedTransport<T, CAPACITY>,
        ) -> Pin<Box<dyn Future<Output = io::Result<()>>>>,
    {
        Self(Box::new(f))
    }
}

impl<T, F, const CAPACITY: usize> From<F> for OnHandshake<T, CAPACITY>
where
    F: FnMut(&mut FramedTransport<T, CAPACITY>) -> Pin<Box<dyn Future<Output = io::Result<()>>>>,
{
    fn from(f: F) -> Self {
        Self::new(f)
    }
}

impl<T, const CAPACITY: usize> fmt::Debug for OnHandshake<T, CAPACITY> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OnHandshake").finish()
    }
}

impl<T, const CAPACITY: usize> Default for OnHandshake<T, CAPACITY> {
    /// Implements handshake callback that does nothing
    fn default() -> Self {
        Self::new(|_| Box::pin(async { Ok(()) }))
    }
}
