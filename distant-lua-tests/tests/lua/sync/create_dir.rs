use crate::common::{fixtures::*, lua, session};
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

    // Make a path that has multiple non-existent components
    // so the creation will fail
    let root_dir = setup_dir();
    let path = root_dir.path().join("nested").join("new-dir");
    let path_str = path.to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local status, _ = pcall(session.create_dir_sync, session, { path = $path_str })
            assert(not status, "Unexpectedly succeeded")
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

    let root_dir = setup_dir();
    let path = root_dir.path().join("new-dir");
    let path_str = path.to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            session:create_dir_sync({ path = $path_str })
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

    let root_dir = setup_dir();
    let path = root_dir.path().join("nested").join("new-dir");
    let path_str = path.to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            session:create_dir_sync({ path = $path_str, all = true })
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Also verify that the directory was actually created
    assert!(path.exists(), "Directory not created");
}
