use crate::common::{fixtures::*, lua};
use mlua::{chunk, prelude::*};
use rstest::*;

#[rstest]
fn some_test(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let addr = ctx.addr;
    let key = ctx.key.clone();

    let result = lua
        .load(chunk! {
            local distant = require("distant_lua")
            local x = 1+1
        })
        .exec();
    assert!(result.is_ok(), "Failed: {:?}", result);
}
