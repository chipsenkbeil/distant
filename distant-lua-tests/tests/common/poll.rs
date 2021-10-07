use mlua::{chunk, prelude::*};
use std::{thread, time::Duration};

/// Creates a function that can be passed as the schedule function for `wrap_async`
pub fn make_function(lua: &Lua) -> LuaResult<LuaFunction> {
    let sleep = lua.create_function(|_, ()| {
        thread::sleep(Duration::from_millis(10));
        Ok(())
    })?;

    lua.load(chunk! {
        local cb = ...
        $sleep()
        cb()
    })
    .into_function()
}
