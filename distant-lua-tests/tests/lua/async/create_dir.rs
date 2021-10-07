use crate::common::{fixtures::*, lua, poll, session};
use assert_fs::prelude::*;
use mlua::chunk;
use rstest::*;

// /root/
// /root/file1
// /root/link1 -> /root/sub1/file2
// /root/sub1/
// /root/sub1/file2
fn setup_dir() -> assert_fs::TempDir {
    let root_dir = assert_fs::TempDir::new().unwrap();
    root_dir.child("file1").touch().unwrap();

    let sub1 = root_dir.child("sub1");
    sub1.create_dir_all().unwrap();

    let file2 = sub1.child("file2");
    file2.touch().unwrap();

    let link1 = root_dir.child("link1");
    link1.symlink_to_file(file2.path()).unwrap();

    root_dir
}

#[rstest]
fn should_send_error_if_fails(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    // Make a path that has multiple non-existent components
    // so the creation will fail
    let root_dir = setup_dir();
    let path = root_dir.path().join("nested").join("new-dir");
    let path_str = path.to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local f = require("distant_lua").utils.wrap_async(
                session.create_dir_async,
                $schedule_fn
            )

            // Because of our scheduler, the invocation turns async -> sync
            local err
            f(session, { path = $path_str }, function(success, res)
                if not success then
                    err = res
                end
            end)
            assert(err, "Unexpectedly succeeded")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Also verify that the directory was not actually created
    assert!(!path.exists(), "Path unexpectedly exists");
}

#[rstest]
fn should_send_ok_when_successful(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let root_dir = setup_dir();
    let path = root_dir.path().join("new-dir");
    let path_str = path.to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local f = require("distant_lua").utils.wrap_async(
                session.create_dir_async,
                $schedule_fn
            )

            // Because of our scheduler, the invocation turns async -> sync
            local err
            f(session, { path = $path_str }, function(success, res)
                if not success then
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Also verify that the directory was actually created
    assert!(path.exists(), "Directory not created");
}

#[rstest]
fn should_support_creating_multiple_dir_components(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let root_dir = setup_dir();
    let path = root_dir.path().join("nested").join("new-dir");
    let path_str = path.to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local f = require("distant_lua").utils.wrap_async(
                session.create_dir_async,
                $schedule_fn
            )

            // Because of our scheduler, the invocation turns async -> sync
            local err
            f(session, { path = $path_str, all = true }, function(success, res)
                if not success then
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Also verify that the directory was actually created
    assert!(path.exists(), "Directory not created");
}
