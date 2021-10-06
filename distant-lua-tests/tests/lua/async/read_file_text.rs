use crate::common::{fixtures::*, lua, poll, session};
use assert_fs::prelude::*;
use mlua::chunk;
use rstest::*;

#[rstest]
fn should_return_error_if_fails_to_read_file(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("missing-file");
    let file_path = file.path();
    let file_path_str = file_path.to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local f = require("distant_lua").utils.wrap_async(
                session.read_file_text_async,
                $schedule_fn
            )

            // Because of our scheduler, the invocation turns async -> sync
            local err
            f(session, { path = $file_path_str }, function(success, res)
                if not success then
                    err = res
                end
            end)
            assert(err, "Unexpectedly succeeded")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

#[rstest]
fn should_return_file_contents_as_byte_list(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("some file contents").unwrap();
    let file_path = file.path();
    let file_path_str = file_path.to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local contents = session:read_file({ path = $file_path_str })
            local f = require("distant_lua").utils.wrap_async(
                session.read_file_text_async,
                $schedule_fn
            )

            // Because of our scheduler, the invocation turns async -> sync
            local err, contents
            f(session, { path = $file_path_str }, function(success, res)
                if success then
                    contents = res
                else
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed")
            assert(contents, "Missing file contents")
            assert(contents == "some file contents", "Unexpected file contents: " .. contents)
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}
