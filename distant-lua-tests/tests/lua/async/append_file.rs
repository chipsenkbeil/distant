use crate::common::{fixtures::*, lua, poll, session};
use assert_fs::prelude::*;
use mlua::chunk;
use predicates::prelude::*;
use rstest::*;

#[rstest]
fn should_yield_error_if_fails_to_create_file(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    // Create a temporary path and add to it to ensure that there are
    // extra components that don't exist to cause writing to fail
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("dir").child("test-file");
    let file_path = file.path().to_str().unwrap();
    let data = b"some text".to_vec();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local f = require("distant_lua").utils.wrap_async(session.append_file, $schedule_fn)

            // Because of our scheduler, the invocation turns async -> sync
            local err
            f(function(success, res)
                if not success then
                    err = res
                end
            end, session, {
                path = $file_path,
                data = $data
            })
            assert(err, "Unexpectedly succeeded")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Also verify that we didn't actually create the file
    file.assert(predicate::path::missing());
}

#[rstest]
fn should_append_data_to_existing_file(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("line 1").unwrap();

    let file_path = file.path().to_str().unwrap();
    let data = b"some text".to_vec();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local f = require("distant_lua").utils.wrap_async(session.append_file, $schedule_fn)

            // Because of our scheduler, the invocation turns async -> sync
            local err
            f(function(success, res)
                if not success then
                    err = res
                end
            end, session, {
                path = $file_path,
                data = $data
            })
            assert(not err, "Unexpectedly failed")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Also verify that we appended to the file
    file.assert("line 1some text");
}
