use crate::common::{fixtures::*, lua, poll, session};
use assert_fs::prelude::*;
use mlua::chunk;
use predicates::prelude::*;
use rstest::*;

#[rstest]
fn should_return_error_on_failure(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let src = temp.child("src");
    let dst = temp.child("dst");
    let src_path = src.path().to_str().unwrap();
    let dst_path = dst.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local f = require("distant_lua").utils.wrap_async(
                session.rename_async,
                $schedule_fn
            )

            // Because of our scheduler, the invocation turns async -> sync
            local err
            f(session, { src = $src_path, dst = $dst_path }, function(success, res)
                if not success then
                    err = res
                end
            end)
            assert(err, "Unexpectedly succeeded")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Also, verify that destination does not exist
    dst.assert(predicate::path::missing());
}

#[rstest]
fn should_support_renaming_an_entire_directory(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();

    let src = temp.child("src");
    src.create_dir_all().unwrap();
    let src_file = src.child("file");
    src_file.write_str("some contents").unwrap();

    let dst = temp.child("dst");
    let dst_file = dst.child("file");

    let src_path = src.path().to_str().unwrap();
    let dst_path = dst.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local f = require("distant_lua").utils.wrap_async(
                session.rename_async,
                $schedule_fn
            )

            // Because of our scheduler, the invocation turns async -> sync
            local err
            f(session, { src = $src_path, dst = $dst_path }, function(success, res)
                if not success then
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Verify that we moved the contents
    src.assert(predicate::path::missing());
    src_file.assert(predicate::path::missing());
    dst.assert(predicate::path::is_dir());
    dst_file.assert("some contents");
}

#[rstest]
fn should_support_renaming_a_single_file(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let src = temp.child("src");
    src.write_str("some text").unwrap();
    let dst = temp.child("dst");

    let src_path = src.path().to_str().unwrap();
    let dst_path = dst.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local f = require("distant_lua").utils.wrap_async(
                session.rename_async,
                $schedule_fn
            )

            // Because of our scheduler, the invocation turns async -> sync
            local err
            f(session, { src = $src_path, dst = $dst_path }, function(success, res)
                if not success then
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Verify that we moved the file
    src.assert(predicate::path::missing());
    dst.assert("some text");
}
