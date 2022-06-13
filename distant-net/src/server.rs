/* use crate::{Codec, RawTransport, Request};
use std::{io, sync::Arc}; */
/*
pub trait ServerHandler {
    type Input;
    type Output;

    fn on_input(input: Self::Input) -> io::Result<Self::Output>;
}

/* pub struct ServerCtx<I, O> {
    pub
} */

pub struct Server<I, O, S> {
    ctx: ServerCtx<I, O>,
    state: Arc<ServerState>,
}

impl<I, O, S> Server<I, O, S> {
    pub fn new<T, C, H>(transport: T, codec: C, handler: H) -> Self
    where
        T: Transport,
        C: Codec,
        H: ServerHandler,
    {
    }
} */
