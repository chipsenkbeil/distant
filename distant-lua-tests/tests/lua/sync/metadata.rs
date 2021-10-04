use crate::common::{fixtures::*, lua, session};
use assert_fs::prelude::*;
use mlua::chunk;
use rstest::*;

#[rstest]
fn should_send_error_on_failure(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    let file_path = file.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local status, _ = pcall(session.metadata_sync, session, { path = $file_path })
            assert(not status, "Unexpectedly succeeded")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

#[rstest]
fn should_return_metadata_on_file_if_exists(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();
    let file_path = file.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local metadata = session:metadata_sync({ path = $file_path })
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

    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("dir");
    dir.create_dir_all().unwrap();
    let dir_path = dir.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local metadata = session:metadata_sync({ path = $dir_path })
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

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();

    let symlink = temp.child("link");
    symlink.symlink_to_file(file.path()).unwrap();
    let symlink_path = symlink.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local metadata = session:metadata_sync({ path = $symlink_path })
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
            local metadata = session:metadata_sync({
                path = $symlink_path,
                canonicalize = true,
            })
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

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.write_str("some text").unwrap();

    let symlink = temp.child("link");
    symlink.symlink_to_file(file.path()).unwrap();
    let symlink_path = symlink.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local metadata = session:metadata_sync({
                path = $symlink_path,
                resolve_file_type = true,
            })
            assert(metadata.file_type == "file", "Got wrong file type: " .. metadata.file_type)
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}
