use crate::common::{fixtures::*, lua, poll, session};
use mlua::chunk;
use rstest::*;

#[rstest]
fn should_return_system_information(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local f = require("distant_lua").utils.wrap_async(
                session.system_info_async,
                $schedule_fn
            )

            // Because of our scheduler, the invocation turns async -> sync
            local err, system_info
            f(session, function(success, res)
                if success then
                    system_info = res
                else
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed")
            assert(system_info, "Missing system information")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}
