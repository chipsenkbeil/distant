use futures::{FutureExt, TryFutureExt};
use mlua::prelude::*;
use once_cell::sync::OnceCell;
use std::future::Future;

/// Retrieves the global runtime, initializing it if not initialized, and returning
/// an error if failed to initialize
pub fn get_runtime() -> LuaResult<&'static tokio::runtime::Runtime> {
    static RUNTIME: OnceCell<tokio::runtime::Runtime> = OnceCell::new();
    RUNTIME.get_or_try_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|x| x.to_lua_err())
    })
}

/// Blocks using the global runtime for a future that returns `LuaResult<T>`
pub fn block_on<F, T>(future: F) -> LuaResult<T>
where
    F: Future<Output = Result<T, LuaError>>,
{
    get_runtime()?.block_on(future)
}

/// Spawns a task on the global runtime for a future that returns a `LuaResult<T>`
pub fn spawn<F, T>(f: F) -> impl Future<Output = LuaResult<T>>
where
    F: Future<Output = Result<T, LuaError>> + Send + 'static,
    T: Send + 'static,
{
    futures::future::ready(get_runtime()).and_then(|rt| {
        rt.spawn(f).map(|result| match result {
            Ok(x) => x.to_lua_err(),
            Err(x) => Err(x).to_lua_err(),
        })
    })
}
