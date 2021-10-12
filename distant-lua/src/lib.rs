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

mod constants;
mod log;
mod runtime;
mod session;
mod utils;

#[mlua::lua_module]
fn distant_lua(lua: &Lua) -> LuaResult<LuaTable> {
    let exports = lua.create_table()?;

    // Provide a static pending type used when consumer wants to use async functions
    // directly without wrapping them with a scheduler
    exports.set("pending", utils::pending(lua)?)?;

    // API modules available for users
    exports.set("log", log::make_log_tbl(lua)?)?;
    exports.set("session", session::make_session_tbl(lua)?)?;
    exports.set("utils", utils::make_utils_tbl(lua)?)?;
    exports.set("version", make_version_tbl(lua)?)?;

    Ok(exports)
}

macro_rules! set_nonempty_env {
    ($tbl:ident, $key:literal, $env_key:literal) => {{
        let value = env!($env_key);
        if !value.is_empty() {
            $tbl.set($key, value)?;
        }
    }};
}

fn make_version_tbl(lua: &Lua) -> LuaResult<LuaTable> {
    let tbl = lua.create_table()?;

    set_nonempty_env!(tbl, "full", "CARGO_PKG_VERSION");
    set_nonempty_env!(tbl, "major", "CARGO_PKG_VERSION_MAJOR");
    set_nonempty_env!(tbl, "minor", "CARGO_PKG_VERSION_MINOR");
    set_nonempty_env!(tbl, "patch", "CARGO_PKG_VERSION_PATCH");
    set_nonempty_env!(tbl, "pre", "CARGO_PKG_VERSION_PRE");

    Ok(tbl)
}
