use crate::common::{fixtures::*, lua, session};
use assert_fs::prelude::*;
use mlua::chunk;
use predicates::prelude::*;
use rstest::*;

#[rstest]
fn should_return_error_on_failure(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("missing-file");
    let file_path = file.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local status, _ = pcall(session.remove, session, { path = $file_path })
            assert(not status, "Unexpectedly succeeded!")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Also, verify that path does not exist
    file.assert(predicate::path::missing());
}

#[rstest]
fn should_support_deleting_a_directory(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();
    let dir_path = dir.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            session:remove({ path = $dir_path })
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Also, verify that path does not exist
    dir.assert(predicate::path::missing());
}

#[rstest]
fn should_delete_nonempty_directory_if_force_is_true(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();
    dir.child("file").touch().unwrap();
    let dir_path = dir.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            session:remove({ path = $dir_path, force = true })
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Also, verify that path does not exist
    dir.assert(predicate::path::missing());
}

#[rstest]
fn should_support_deleting_a_single_file(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("some-file");
    file.touch().unwrap();
    let file_path = file.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            session:remove({ path = $file_path })
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Also, verify that path does not exist
    file.assert(predicate::path::missing());
}
