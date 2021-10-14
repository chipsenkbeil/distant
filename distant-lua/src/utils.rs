use crate::constants::NVIM_POLL_TIMEOUT;
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
        "nvim_wrap_async",
        lua.create_function(|lua, (async_fn, millis): (LuaFunction, Option<u64>)| {
            nvim_wrap_async(lua, async_fn, millis.unwrap_or(NVIM_POLL_TIMEOUT))
        })?,
    )?;
    tbl.set(
        "wrap_async",
        lua.create_function(|lua, (async_fn, schedule_fn)| wrap_async(lua, async_fn, schedule_fn))?,
    )?;
    tbl.set("rand_u32", lua.create_function(|_, ()| rand_u32())?)?;

    Ok(tbl)
}

/// Specialty function that performs wrap_async using `vim.defer_fn` from neovim
pub fn nvim_wrap_async<'a>(
    lua: &'a Lua,
    async_fn: LuaFunction<'a>,
    millis: u64,
) -> LuaResult<LuaFunction<'a>> {
    let schedule_fn = lua
        .load(chunk! {
            function(cb)
                return vim.defer_fn(cb, $millis)
            end
        })
        .eval()?;
    wrap_async(lua, async_fn, schedule_fn)
}

/// Wraps an async function and a scheduler function such that
/// a new function is returned that takes a callback when the async
/// function completes as well as zero or more arguments to provide
/// to the async function when first executing it
///
/// ```lua
/// local f = wrap_async(some_async_fn, schedule_fn)
/// f(arg1, arg2, ..., function(success, res) end)
/// ```
pub fn wrap_async<'a>(
    lua: &'a Lua,
    async_fn: LuaFunction<'a>,
    schedule_fn: LuaFunction<'a>,
) -> LuaResult<LuaFunction<'a>> {
    let pending = pending(lua)?;
    lua.load(chunk! {
        return function(...)
            local args = {...}
            local cb = table.remove(args)

            assert(type(cb) == "function", "Invalid type for cb")
            local schedule = function(...) return $schedule_fn(...) end

            // Wrap the async function in a coroutine so we can poll it
            local thread = coroutine.create(function(...) return $async_fn(...) end)

            // Start the future by peforming the first poll
            local status, res = coroutine.resume(thread, unpack(args))

            local inner_fn
            inner_fn = function()
                // Thread has exited already, so res is an error
                if not status then
                    cb(false, res)
                // Got pending status on success, so we are still waiting
                elseif res == $pending then
                    // Resume the coroutine and then schedule a followup
                    // once it has completed another round
                    status, res = coroutine.resume(thread)
                    schedule(inner_fn)
                // Got success with non-pending status, so this should be the result
                else
                    cb(true, res)
                end
            end
            schedule(inner_fn)
        end
    })
    .eval()
}

/// Return mlua's internal `Poll::Pending`
pub(super) fn pending(lua: &Lua) -> LuaResult<LuaValue> {
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
