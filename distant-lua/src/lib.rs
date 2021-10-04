use mlua::prelude::*;

mod runtime;
mod session;
mod utils;

#[mlua::lua_module]
fn distant_lua(lua: &Lua) -> LuaResult<LuaTable> {
    let exports = lua.create_table()?;

    exports.set("PENDING", utils::pending(lua)?)?;
    exports.set("session", session::make_session_tbl(lua)?)?;
    exports.set("utils", utils::make_utils_tbl(lua)?)?;

    Ok(exports)
}
