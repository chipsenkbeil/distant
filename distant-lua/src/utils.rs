use mlua::{chunk, prelude::*};
use once_cell::sync::OnceCell;
use oorandom::Rand32;
use std::{
    sync::Mutex,
    time::{SystemTime, SystemTimeError, UNIX_EPOCH},
};

/// Makes a Lua table containing the utils functions
pub fn make_utils_tbl(lua: &Lua) -> LuaResult<LuaTable> {
    let tbl = lua.create_table()?;

    tbl.set(
        "make_callback",
        lua.create_function(|lua, (async_fn, schedule_fn)| {
            make_callback(lua, async_fn, schedule_fn)
        })?,
    )?;
    tbl.set("pending", lua.create_function(|lua, ()| pending(lua))?)?;
    tbl.set("rand_u32", lua.create_function(|_, ()| rand_u32())?)?;

    Ok(tbl)
}

pub fn make_callback<'a>(
    lua: &'a Lua,
    async_fn: LuaFunction<'a>,
    schedule_fn: LuaFunction<'a>,
) -> LuaResult<LuaFunction<'a>> {
    let pending = pending(lua)?;
    lua.load(chunk! {
        function(args, cb)
            assert(type(args) == "table", "Invalid type for args")
            assert(type(cb) == "function", "Invalid type for cb")

            local poll_pending = (coroutine.wrap($pending))()
            local thread_fn = coroutine.wrap($async_fn)
            local status, res = pcall(thread_fn(args))

            local inner_fn
            inner_fn = function()
                if res == poll_pending then
                    $schedule_fn(inner_fn)
                else
                    cb(res)
                end
            end
            $schedule_fn(inner_fn)
        end
    })
    .eval()
}

/// Return mlua's internal `Poll::Pending`
pub fn pending(lua: &Lua) -> LuaResult<LuaValue> {
    let pending = lua.create_async_function(|_, ()| async move {
        tokio::task::yield_now().await;
        Ok(())
    })?;

    // Should return mlua's internal Poll::Pending that is statically available
    // See https://github.com/khvzak/mlua/issues/76#issuecomment-932645078
    lua.load(chunk! {
        (coroutine.wrap($pending))()
    })
    .eval()
}

/// Return a random u32
pub fn rand_u32() -> LuaResult<u32> {
    static RAND: OnceCell<Mutex<Rand32>> = OnceCell::new();

    Ok(RAND
        .get_or_try_init::<_, SystemTimeError>(|| {
            Ok(Mutex::new(Rand32::new(
                SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            )))
        })
        .to_lua_err()?
        .lock()
        .map_err(|x| x.to_string())
        .to_lua_err()?
        .rand_u32())
}
