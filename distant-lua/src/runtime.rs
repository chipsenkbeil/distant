use mlua::prelude::*;
use once_cell::sync::OnceCell;

/// Retrieves the global runtime, initializing it if not initialized, and returning
/// an error if failed to initialize
pub fn get_runtime() -> LuaResult<&'static tokio::runtime::Runtime> {
    static RUNTIME: OnceCell<tokio::runtime::Runtime> = OnceCell::new();
    RUNTIME.get_or_try_init(|| {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|x| x.to_lua_err())
    })
}
