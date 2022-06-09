macro_rules! impl_async_newtype {
    (@read $name:ident) => {
        impl tokio::io::AsyncRead for $name {
            fn poll_read(
                self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
                buf: &mut tokio::io::ReadBuf<'_>,
            ) -> std::task::Poll<io::Result<()>> {
                std::pin::Pin::new(std::pin::Pin::get_mut(self)).poll_read(cx, buf)
            }
        }
    };
    (@write $name:ident) => {
        impl tokio::io::AsyncWrite for $name {
            fn poll_write(
                self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
                buf: &[u8],
            ) -> std::task::Poll<Result<usize, tokio::io::Error>> {
                std::pin::Pin::new(std::pin::Pin::get_mut(self)).poll_write(cx, buf)
            }

            fn poll_flush(
                self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Result<(), tokio::io::Error>> {
                std::pin::Pin::new(std::pin::Pin::get_mut(self)).poll_flush(cx)
            }

            fn poll_shutdown(
                self: std::pin::Pin<&mut Self>,
                cx: &mut std::task::Context<'_>,
            ) -> std::task::Poll<Result<(), tokio::io::Error>> {
                std::pin::Pin::new(std::pin::Pin::get_mut(self)).poll_shutdown(cx)
            }
        }
    };
    ($name:ident -> $inner:path) => {
        #[derive(
            derive_more::AsMut,
            derive_more::AsRef,
            derive_more::Deref,
            derive_more::DerefMut,
            derive_more::From,
            derive_more::Into
        )]
        pub struct $name($inner);
        impl_async_newtype!(@read $name);
        impl_async_newtype!(@write $name);
    };
}

mod codec;
mod key;
mod listener;
mod transport;

pub use codec::*;
pub use key::*;
pub use listener::*;
pub use transport::*;
