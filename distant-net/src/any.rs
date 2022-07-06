use std::any::Any;

/// Trait used for casting support into the [`Any`] trait object
pub trait AsAny: Any {
    /// Converts reference to [`Any`]
    fn as_any(&self) -> &dyn Any;

    /// Converts mutable reference to [`Any`]
    fn as_mut_any(&mut self) -> &mut dyn Any;

    /// Consumes and produces `Box<dyn Any>`
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
}

/// Blanket implementation that enables any `'static` reference to convert
/// to the [`Any`] type
impl<T: 'static> AsAny for T {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_mut_any(&mut self) -> &mut dyn Any {
        self
    }

    fn into_any(self: Box<Self>) -> Box<dyn Any> {
        self
    }
}
