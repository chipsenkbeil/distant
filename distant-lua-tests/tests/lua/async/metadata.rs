use crate::common::{fixtures::*, lua, poll, session};
use assert_fs::prelude::*;
use mlua::chunk;
use rstest::*;

#[rstest]
fn should_send_error_on_failure(ctx: &'_ DistantServerCtx) {
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
                session.metadata,
                $schedule_fn
            )

            // Because of our scheduler, the invocation turns async -> sync
            local err
            f(session, { path = $file_path }, function(success, res)
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
fn should_return_metadata_on_file_if_exists(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();
    let file_path = file.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local f = require("distant_lua").utils.wrap_async(
                session.metadata,
                $schedule_fn
            )

            // Because of our scheduler, the invocation turns async -> sync
            local err, metadata
            f(session, { path = $file_path }, function(success, res)
                if success then
                    metadata = res
                else
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed")
            assert(metadata, "Missing metadata")
            assert(not metadata.canonicalized_path, "Unexpectedly got canonicalized path")
            assert(metadata.file_type == "file", "Got wrong file type: " .. metadata.file_type)
            assert(metadata.len == 9, "Got wrong len: " .. metadata.len)
            assert(not metadata.readonly, "Unexpectedly readonly")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

#[rstest]
fn should_return_metadata_on_dir_if_exists(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();
    let dir_path = dir.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local f = require("distant_lua").utils.wrap_async(
                session.metadata,
                $schedule_fn
            )

            // Because of our scheduler, the invocation turns async -> sync
            local err, metadata
            f(session, { path = $dir_path }, function(success, res)
                if success then
                    metadata = res
                else
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed")
            assert(metadata, "Missing metadata")
            assert(not metadata.canonicalized_path, "Unexpectedly got canonicalized path")
            assert(metadata.file_type == "dir", "Got wrong file type: " .. metadata.file_type)
            assert(not metadata.readonly, "Unexpectedly readonly")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

#[rstest]
fn should_return_metadata_on_symlink_if_exists(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();

    let symlink = temp.child("link");
    symlink.symlink_to_file(file.path()).unwrap();
    let symlink_path = symlink.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local f = require("distant_lua").utils.wrap_async(
                session.metadata,
                $schedule_fn
            )

            // Because of our scheduler, the invocation turns async -> sync
            local err, metadata
            f(session, { path = $symlink_path }, function(success, res)
                if success then
                    metadata = res
                else
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed")
            assert(metadata, "Missing metadata")
            assert(not metadata.canonicalized_path, "Unexpectedly got canonicalized path")
            assert(metadata.file_type == "symlink", "Got wrong file type: " .. metadata.file_type)
            assert(not metadata.readonly, "Unexpectedly readonly")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

#[rstest]
fn should_include_canonicalized_path_if_flag_specified(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();
    let file_path = file.path().canonicalize().unwrap();
    let file_path_str = file_path.to_str().unwrap();

    let symlink = temp.child("link");
    symlink.symlink_to_file(file.path()).unwrap();
    let symlink_path = symlink.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local f = require("distant_lua").utils.wrap_async(
                session.metadata,
                $schedule_fn
            )

            // Because of our scheduler, the invocation turns async -> sync
            local err, metadata
            f(session, { path = $symlink_path, canonicalize = true }, function(success, res)
                if success then
                    metadata = res
                else
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed")
            assert(metadata, "Missing metadata")
            assert(
                metadata.canonicalized_path == $file_path_str,
                "Got wrong canonicalized path: " .. metadata.canonicalized_path
            )
            assert(metadata.file_type == "symlink", "Got wrong file type: " .. metadata.file_type)
            assert(not metadata.readonly, "Unexpectedly readonly")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

#[rstest]
fn should_resolve_file_type_of_symlink_if_flag_specified(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();

    let symlink = temp.child("link");
    symlink.symlink_to_file(file.path()).unwrap();
    let symlink_path = symlink.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local f = require("distant_lua").utils.wrap_async(
                session.metadata,
                $schedule_fn
            )

            // Because of our scheduler, the invocation turns async -> sync
            local err, metadata
            f(session, { path = $symlink_path, resolve_file_type = true }, function(success, res)
                if success then
                    metadata = res
                else
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed")
            assert(metadata, "Missing metadata")
            assert(metadata.file_type == "file", "Got wrong file type: " .. metadata.file_type)
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}
