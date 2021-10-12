use super::fixtures::DistantServerCtx;
use mlua::{chunk, prelude::*};

/// Creates a function that produces a session within the provided Lua environment
/// using the given distant server context, returning the session's id
pub fn make_function<'a>(lua: &'a Lua, ctx: &'_ DistantServerCtx) -> LuaResult<LuaFunction<'a>> {
    let addr = ctx.addr;
    let host = addr.ip().to_string();
    let port = addr.port();
    let key = ctx.key.clone();

    lua.load(chunk! {
        local distant = require("distant_lua")
        local thread = coroutine.create(distant.session.connect_async)

        local status, res = coroutine.resume(thread, {
            host = $host,
            port = $port,
            key = $key,
            timeout = 15000,
        })

        // Block until the connection finishes
        local session = nil
        while status do
            if status and res ~= distant.pending then
                session = res
                break
            end

            status, res = coroutine.resume(thread)
        end

        if session then
            return session
        else
            error(res)
        end
    })
    .into_function()
}
