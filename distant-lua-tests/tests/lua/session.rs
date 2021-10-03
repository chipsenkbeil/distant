use crate::common::{fixtures::*, lua, session};
use mlua::{chunk, prelude::*};
use rstest::*;

#[rstest]
fn some_test(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let id = session::make(&lua, ctx).unwrap();

    let result = lua
        .load(chunk! {
            local distant = require("distant_lua")
            local session = distant.get_session_by_id($id)
            print("Session: " .. session.id)
        })
        .exec();
    assert!(result.is_ok(), "Failed: {:?}", result);
}
