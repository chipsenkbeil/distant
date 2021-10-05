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
fn should_return_error_if_directory_does_not_exist(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("test-dir");
    let dir_path = dir.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local status, _ = pcall(session.read_dir, session, { path = $dir_path })
            assert(not status, "Unexpectedly succeeded")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

#[rstest]
fn should_have_depth_default_to_1(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    // Create directory with some nested items
    let root_dir = setup_dir();
    let root_dir_path = root_dir.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local tbl = session:read_dir({ path = $root_dir_path })

            local entries = tbl.entries
            assert(entries[1].file_type == "file", "Wrong file type: " .. entries[1].file_type)
            assert(entries[1].path == "file1", "Wrong path: " .. entries[1].path)
            assert(entries[1].depth == 1, "Wrong depth")

            assert(entries[2].file_type == "symlink", "Wrong file type: " .. entries[2].file_type)
            assert(entries[2].path == "link1", "Wrong path: " .. entries[2].path)
            assert(entries[2].depth == 1, "Wrong depth")

            assert(entries[3].file_type == "dir", "Wrong file type: " .. entries[3].file_type)
            assert(entries[3].path == "sub1", "Wrong path: " .. entries[3].path)
            assert(entries[3].depth == 1, "Wrong depth")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

#[rstest]
fn should_support_depth_limits(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    // Create directory with some nested items
    let root_dir = setup_dir();
    let root_dir_path = root_dir.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local tbl = session:read_dir({ path = $root_dir_path, depth = 1 })

            local entries = tbl.entries
            assert(entries[1].file_type == "file", "Wrong file type: " .. entries[1].file_type)
            assert(entries[1].path == "file1", "Wrong path: " .. entries[1].path)
            assert(entries[1].depth == 1, "Wrong depth")

            assert(entries[2].file_type == "symlink", "Wrong file type: " .. entries[2].file_type)
            assert(entries[2].path == "link1", "Wrong path: " .. entries[2].path)
            assert(entries[2].depth == 1, "Wrong depth")

            assert(entries[3].file_type == "dir", "Wrong file type: " .. entries[3].file_type)
            assert(entries[3].path == "sub1", "Wrong path: " .. entries[3].path)
            assert(entries[3].depth == 1, "Wrong depth")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

#[rstest]
fn should_support_unlimited_depth_using_zero(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    // Create directory with some nested items
    let root_dir = setup_dir();
    let root_dir_path = root_dir.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local tbl = session:read_dir({ path = $root_dir_path, depth = 0 })

            local entries = tbl.entries
            assert(entries[1].file_type == "file", "Wrong file type: " .. entries[1].file_type)
            assert(entries[1].path == "file1", "Wrong path: " .. entries[1].path)
            assert(entries[1].depth == 1, "Wrong depth")

            assert(entries[2].file_type == "symlink", "Wrong file type: " .. entries[2].file_type)
            assert(entries[2].path == "link1", "Wrong path: " .. entries[2].path)
            assert(entries[2].depth == 1, "Wrong depth")

            assert(entries[3].file_type == "dir", "Wrong file type: " .. entries[3].file_type)
            assert(entries[3].path == "sub1", "Wrong path: " .. entries[3].path)
            assert(entries[3].depth == 1, "Wrong depth")

            assert(entries[4].file_type == "file", "Wrong file type: " .. entries[4].file_type)
            assert(entries[4].path == "sub1/file2", "Wrong path: " .. entries[4].path)
            assert(entries[4].depth == 2, "Wrong depth")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

#[rstest]
fn should_support_including_directory_in_returned_entries(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    // Create directory with some nested items
    let root_dir = setup_dir();
    let root_dir_path = root_dir.path().to_str().unwrap();
    let root_dir_canonicalized_path = root_dir.path().canonicalize().unwrap();
    let root_dir_canonicalized_path_str = root_dir_canonicalized_path.to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local tbl = session:read_dir({ path = $root_dir_path, include_root = true })

            local entries = tbl.entries
            assert(entries[1].file_type == "dir", "Wrong file type: " .. entries[1].file_type)
            assert(entries[1].path == $root_dir_canonicalized_path_str, "Wrong path: " .. entries[1].path)
            assert(entries[1].depth == 0, "Wrong depth")

            assert(entries[2].file_type == "file", "Wrong file type: " .. entries[2].file_type)
            assert(entries[2].path == "file1", "Wrong path: " .. entries[2].path)
            assert(entries[2].depth == 1, "Wrong depth")

            assert(entries[3].file_type == "symlink", "Wrong file type: " .. entries[3].file_type)
            assert(entries[3].path == "link1", "Wrong path: " .. entries[3].path)
            assert(entries[3].depth == 1, "Wrong depth")

            assert(entries[4].file_type == "dir", "Wrong file type: " .. entries[4].file_type)
            assert(entries[4].path == "sub1", "Wrong path: " .. entries[4].path)
            assert(entries[4].depth == 1, "Wrong depth")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

#[rstest]
fn should_support_returning_absolute_paths(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    // Create directory with some nested items
    let root_dir = setup_dir();
    let root_dir_path = root_dir.path().to_str().unwrap();

    let root_dir_canonicalized_path = root_dir.path().canonicalize().unwrap();
    let file1_path = root_dir_canonicalized_path.join("file1");
    let link1_path = root_dir_canonicalized_path.join("link1");
    let sub1_path = root_dir_canonicalized_path.join("sub1");

    let file1_path_str = file1_path.to_str().unwrap();
    let link1_path_str = link1_path.to_str().unwrap();
    let sub1_path_str = sub1_path.to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local tbl = session:read_dir({ path = $root_dir_path, absolute = true })

            local entries = tbl.entries
            assert(entries[1].file_type == "file", "Wrong file type: " .. entries[1].file_type)
            assert(entries[1].path == $file1_path_str, "Wrong path: " .. entries[1].path)
            assert(entries[1].depth == 1, "Wrong depth")

            assert(entries[2].file_type == "symlink", "Wrong file type: " .. entries[2].file_type)
            assert(entries[2].path == $link1_path_str, "Wrong path: " .. entries[2].path)
            assert(entries[2].depth == 1, "Wrong depth")

            assert(entries[3].file_type == "dir", "Wrong file type: " .. entries[3].file_type)
            assert(entries[3].path == $sub1_path_str, "Wrong path: " .. entries[3].path)
            assert(entries[3].depth == 1, "Wrong depth")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

#[rstest]
fn should_support_returning_canonicalized_paths(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    // Create directory with some nested items
    let root_dir = setup_dir();
    let root_dir_path = root_dir.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local tbl = session:read_dir({ path = $root_dir_path, canonicalize = true })

            local entries = tbl.entries
            assert(entries[1].file_type == "file", "Wrong file type: " .. entries[1].file_type)
            assert(entries[1].path == "file1", "Wrong path: " .. entries[1].path)
            assert(entries[1].depth == 1, "Wrong depth")

            assert(entries[2].file_type == "symlink", "Wrong file type: " .. entries[2].file_type)
            assert(entries[2].path == "sub1/file2", "Wrong path: " .. entries[2].path)
            assert(entries[2].depth == 1, "Wrong depth")

            assert(entries[3].file_type == "dir", "Wrong file type: " .. entries[3].file_type)
            assert(entries[3].path == "sub1", "Wrong path: " .. entries[3].path)
            assert(entries[3].depth == 1, "Wrong depth")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}
