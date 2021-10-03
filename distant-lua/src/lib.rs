use async_compat::CompatExt;
use mlua::prelude::*;

mod session;
mod utils;

pub use session::{ConnectOpts, LaunchOpts, Session};

#[mlua::lua_module]
fn distant_lua(lua: &Lua) -> LuaResult<LuaTable> {
    let exports = lua.create_table()?;

    exports.set("PENDING", utils::pending(lua)?)?;
    exports.set("utils", utils::make_utils_tbl(lua)?)?;

    // get_session_by_id(id: usize) -> Session
    exports.set(
        "get_session_by_id",
        lua.create_function(|_, id: usize| Ok(Session::new(id)))?,
    )?;

    // launch(opts: LaunchOpts) -> Session
    exports.set(
        "launch",
        lua.create_async_function(|lua, opts: LuaValue| async move {
            let opts = LaunchOpts::from_lua(opts, lua)?;
            Session::launch(opts).compat().await
        })?,
    )?;

    // connect(opts: ConnectOpts) -> Session
    exports.set(
        "connect",
        lua.create_async_function(|lua, opts: LuaValue| async move {
            let opts = ConnectOpts::from_lua(opts, lua)?;
            Session::connect(opts).compat().await
        })?,
    )?;

    Ok(exports)
}
