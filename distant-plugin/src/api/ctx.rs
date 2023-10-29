use async_trait::async_trait;

/// Type abstraction of a boxed [`Ctx`].
pub type BoxedCtx = Box<dyn Ctx>;

/// Represents a context associated when an API request is being executed, supporting the ability
/// to send responses back asynchronously.
#[async_trait]
pub trait Ctx: Send {}
