use crate::common::{fixtures::*, lua, poll, session};
use assert_fs::prelude::*;
use mlua::chunk;
use rstest::*;

#[rstest]
fn should_send_true_if_path_exists(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.touch().unwrap();
    let file_path = file.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local f = require("distant_lua").utils.wrap_async(
                session.exists,
                $schedule_fn
            )

            // Because of our scheduler, the invocation turns async -> sync
            local err, exists
            f(session, { path = $file_path }, function(success, res)
                if success then
                    exists = res
                else
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed")
            assert(exists == true, "Invalid exists return value")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

#[rstest]
fn should_send_false_if_path_does_not_exist(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    let file_path = file.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local f = require("distant_lua").utils.wrap_async(
                session.exists,
                $schedule_fn
            )

            // Because of our scheduler, the invocation turns async -> sync
            local err, exists
            f(session, { path = $file_path }, function(success, res)
                if success then
                    exists = res
                else
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed")
            assert(exists == false, "Invalid exists return value")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}
