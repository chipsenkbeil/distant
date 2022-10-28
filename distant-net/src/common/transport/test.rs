use super::{Interest, Ready, Reconnectable, Transport};
use async_trait::async_trait;
use std::io;

pub struct TestTransport {
    pub f_try_read: Box<dyn Fn(&mut [u8]) -> io::Result<usize> + Send + Sync>,
    pub f_try_write: Box<dyn Fn(&[u8]) -> io::Result<usize> + Send + Sync>,
    pub f_ready: Box<dyn Fn(Interest) -> io::Result<Ready> + Send + Sync>,
    pub f_reconnect: Box<dyn Fn() -> io::Result<()> + Send + Sync>,
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

#[async_trait]
impl Reconnectable for TestTransport {
    async fn reconnect(&mut self) -> io::Result<()> {
        (self.f_reconnect)()
    }
}

#[async_trait]
impl Transport for TestTransport {
    fn try_read(&self, buf: &mut [u8]) -> io::Result<usize> {
        (self.f_try_read)(buf)
    }

    fn try_write(&self, buf: &[u8]) -> io::Result<usize> {
        (self.f_try_write)(buf)
    }

    async fn ready(&self, interest: Interest) -> io::Result<Ready> {
        (self.f_ready)(interest)
    }
}
