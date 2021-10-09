use crate::common::{fixtures::*, lua, session};
use mlua::chunk;
use rstest::*;

#[rstest]
fn should_return_system_info(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local system_info = session:system_info()
            assert(system_info, "System info unexpectedly missing")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}
