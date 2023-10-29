use std::io;

use async_trait::async_trait;
use distant_core_protocol::Response;

/// Type abstraction of a boxed [`Ctx`].
pub type BoxedCtx = Box<dyn Ctx>;

/// Represents a context associated when an API request is being executed, supporting the ability
/// to send responses back asynchronously.
#[async_trait]
pub trait Ctx: Send {
    /// Id of the connection associated with this context.
    fn connection(&self) -> u32;

    /// Clones context, returning a new boxed instance.
    fn clone_ctx(&self) -> BoxedCtx;

    /// Sends some response back.
    fn send(&self, response: Response) -> io::Result<()>;
}
