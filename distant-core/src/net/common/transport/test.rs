use std::{fmt, io};

use super::{Interest, Ready, Reconnectable, Transport};

pub type TryReadFn = Box<dyn Fn(&mut [u8]) -> io::Result<usize> + Send + Sync>;
pub type TryWriteFn = Box<dyn Fn(&[u8]) -> io::Result<usize> + Send + Sync>;
pub type ReadyFn = Box<dyn Fn(Interest) -> io::Result<Ready> + Send + Sync>;
pub type ReconnectFn = Box<dyn Fn() -> io::Result<()> + Send + Sync>;

pub struct TestTransport {
    pub f_try_read: TryReadFn,
    pub f_try_write: TryWriteFn,
    pub f_ready: ReadyFn,
    pub f_reconnect: ReconnectFn,
}

impl Default for TestTransport {
    fn default() -> Self {
        Self {
            f_try_read: Box::new(|_| unimplemented!()),
            f_try_write: Box::new(|_| unimplemented!()),
            f_ready: Box::new(|_| unimplemented!()),
            f_reconnect: Box::new(|| unimplemented!()),
        }
    }
}

impl fmt::Debug for TestTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TestTransport").finish()
    }
}

impl Reconnectable for TestTransport {
    fn reconnect<'a>(
        &'a mut self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<()>> + Send + 'a>> {
        Box::pin(async move { (self.f_reconnect)() })
    }
}

impl Transport for TestTransport {
    fn try_read(&self, buf: &mut [u8]) -> io::Result<usize> {
        (self.f_try_read)(buf)
    }

    fn try_write(&self, buf: &[u8]) -> io::Result<usize> {
        (self.f_try_write)(buf)
    }

    fn ready<'a>(
        &'a self,
        interest: Interest,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<Ready>> + Send + 'a>> {
        Box::pin(async move { (self.f_ready)(interest) })
    }
}
