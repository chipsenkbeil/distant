use std::ops::{Deref, DerefMut};

/// Wrapper type around `T` that provides compile-time confirmation of being authenticated
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Authenticated<T>(T);

impl<T> Authenticated<T> {
    /// Consumes authenticated wrapper and returns the inner value
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> AsRef<T> for Authenticated<T> {
    fn as_ref(&self) -> &T {
        &self.0
    }
}

impl<T> AsMut<T> for Authenticated<T> {
    fn as_mut(&mut self) -> &mut T {
        &mut self.0
    }
}

impl<T> Deref for Authenticated<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Authenticated<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
