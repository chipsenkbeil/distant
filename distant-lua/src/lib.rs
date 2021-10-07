use mlua::prelude::*;

/// to_value!<'a, T: Serialize + ?Sized>(lua: &'a Lua, t: &T) -> Result<Value<'a>>
///
/// Converts to a Lua value using options specific to this module.
macro_rules! to_value {
    ($lua:expr, $x:expr) => {{
        use mlua::{prelude::*, LuaSerdeExt};
        let options = LuaSerializeOptions::new()
            .serialize_none_to_null(false)
            .serialize_unit_to_null(false);
        $lua.to_value_with($x, options)
    }};
}

mod log;
mod runtime;
mod session;
mod utils;

#[mlua::lua_module]
fn distant_lua(lua: &Lua) -> LuaResult<LuaTable> {
    let exports = lua.create_table()?;

    exports.set("PENDING", utils::pending(lua)?)?;
    exports.set("log", log::make_log_tbl(lua)?)?;
    exports.set("session", session::make_session_tbl(lua)?)?;
    exports.set("utils", utils::make_utils_tbl(lua)?)?;

    Ok(exports)
}
