use super::fixtures::DistantServerCtx;
use mlua::{chunk, prelude::*};

/// Creates a new session within the provided Lua environment
/// using the given distant server context, returning the session's id
pub fn make(lua: &Lua, ctx: &'_ DistantServerCtx) -> LuaResult<usize> {
    let addr = ctx.addr;
    let host = addr.ip().to_string();
    let port = addr.port();
    let key = ctx.key.clone();

    lua.load(chunk! {
        (function()
            local distant = require("distant_lua")
            local connect = coroutine.wrap(distant.connect)

            local status, res = pcall(connect, {
                host = "127.0.0.1",
                port = 22,
                key = "Bad key",
                timeout = 15000,
            })
        end)()
    })
    .eval()

    /* local status, res = coroutine.resume(thread, {
        host = $host,
        port = $port,
        key = $key,
        timeout = 15000,
    })

    // Block until the connection finishes
    local session = nil
    while status do
        if status and res ~= distant.PENDING then
            session = res
            break
        end

        status, res = coroutine.resume(thread)
    end

    if session then
        return session.id
    end */
}
