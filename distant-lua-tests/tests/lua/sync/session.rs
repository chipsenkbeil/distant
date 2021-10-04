use crate::common::{fixtures::*, lua, session};
use assert_fs::prelude::*;
use mlua::{chunk, prelude::*};
use predicates::prelude::*;
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
fn append_file_sync_should_yield_error_if_fails_to_create_file(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    // Create a temporary path and add to it to ensure that there are
    // extra components that don't exist to cause writing to fail
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("dir").child("test-file");
    let file_path = file.path().to_str().unwrap();
    let data = b"some text".to_vec();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local status, _ = pcall(session.append_file_sync, session, {
                path = $file_path,
                data = $data
            })
            assert(not status, "Unexpectedly succeeded!")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Also verify that we didn't actually create the file
    file.assert(predicate::path::missing());
}

#[rstest]
fn append_file_sync_should_append_data_to_existing_file(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    // Create a temporary path and add to it to ensure that there are
    // extra components that don't exist to cause writing to fail
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("line 1").unwrap();

    let file_path = file.path().to_str().unwrap();
    let data = b"some text".to_vec();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            session:append_file_sync({
                path = $file_path,
                data = $data
            })
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Also verify that we appended to the file
    file.assert("line 1some text");
}

#[rstest]
fn append_file_text_sync_should_yield_error_if_fails_to_create_file(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    // Create a temporary path and add to it to ensure that there are
    // extra components that don't exist to cause writing to fail
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("dir").child("test-file");
    let file_path = file.path().to_str().unwrap();
    let text = "some text".to_string();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local status, _ = pcall(session.append_file_text_sync, session, {
                path = $file_path,
                data = $text
            })
            assert(not status, "Unexpectedly succeeded!")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Also verify that we didn't actually create the file
    file.assert(predicate::path::missing());
}

#[rstest]
fn append_file_text_sync_should_append_data_to_existing_file(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    // Create a temporary path and add to it to ensure that there are
    // extra components that don't exist to cause writing to fail
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("line 1").unwrap();

    let file_path = file.path().to_str().unwrap();
    let text = "some text".to_string();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            session:append_file_text_sync({
                path = $file_path,
                data = $text
            })
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Also verify that we appended to the file
    file.assert("line 1some text");
}

#[rstest]
fn copy_sync_should_send_error_on_failure(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let src = temp.child("src");
    let dst = temp.child("dst");

    let src_path = src.path().to_str().unwrap();
    let dst_path = dst.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local status, _ = pcall(session.copy_sync, session, {
                src = $src_path,
                dst = $dst_path
            })
            assert(not status, "Unexpectedly succeeded!")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Also, verify that destination does not exist
    dst.assert(predicate::path::missing());
}

#[rstest]
fn copy_sync_should_support_copying_an_entire_directory(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

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
            session:copy_sync({
                src = $src_path,
                dst = $dst_path
            })
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Verify that we have source and destination directories and associated contents
    src.assert(predicate::path::is_dir());
    src_file.assert(predicate::path::is_file());
    dst.assert(predicate::path::is_dir());
    dst_file.assert(predicate::path::eq_file(src_file.path()));
}

#[rstest]
fn copy_sync_should_support_copying_an_empty_directory(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let src = temp.child("src");
    src.create_dir_all().unwrap();
    let dst = temp.child("dst");

    let src_path = src.path().to_str().unwrap();
    let dst_path = dst.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            session:copy_sync({
                src = $src_path,
                dst = $dst_path
            })
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Verify that we still have source and destination directories
    src.assert(predicate::path::is_dir());
    dst.assert(predicate::path::is_dir());
}

#[rstest]
fn copy_sync_should_support_copying_a_directory_that_only_contains_directories(
    ctx: &'_ DistantServerCtx,
) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let src = temp.child("src");
    src.create_dir_all().unwrap();
    let src_dir = src.child("dir");
    src_dir.create_dir_all().unwrap();

    let dst = temp.child("dst");
    let dst_dir = dst.child("dir");

    let src_path = src.path().to_str().unwrap();
    let dst_path = dst.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            session:copy_sync({
                src = $src_path,
                dst = $dst_path
            })
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Verify that we have source and destination directories and associated contents
    src.assert(predicate::path::is_dir().name("src"));
    src_dir.assert(predicate::path::is_dir().name("src/dir"));
    dst.assert(predicate::path::is_dir().name("dst"));
    dst_dir.assert(predicate::path::is_dir().name("dst/dir"));
}

#[rstest]
fn copy_sync_should_support_copying_a_single_file(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let src = temp.child("src");
    src.write_str("some text").unwrap();
    let dst = temp.child("dst");

    let src_path = src.path().to_str().unwrap();
    let dst_path = dst.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            session:copy_sync({
                src = $src_path,
                dst = $dst_path
            })
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Verify that we still have source and that destination has source's contents
    src.assert(predicate::path::is_file());
    dst.assert(predicate::path::eq_file(src.path()));
}

#[rstest]
fn create_dir_sync_should_send_error_if_fails(ctx: &'_ DistantServerCtx) {
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
fn create_dir_sync_should_send_ok_when_successful(ctx: &'_ DistantServerCtx) {
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
fn create_dir_sync_should_support_creating_multiple_dir_components(ctx: &'_ DistantServerCtx) {
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

#[rstest]
fn exists_sync_should_send_true_if_path_exists(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.touch().unwrap();
    let file_path = file.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local exists = session:exists_sync({ path = $file_path })
            assert(exists, "File unexpectedly missing")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

#[rstest]
fn exists_sync_should_send_false_if_path_does_not_exist(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    let file_path = file.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local exists = session:exists_sync({ path = $file_path })
            assert(not exists, "File unexpectedly found")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

#[rstest]
fn metadata_should_send_error_on_failure(ctx: &'_ DistantServerCtx) {
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
fn metadata_should_return_metadata_on_file_if_exists(ctx: &'_ DistantServerCtx) {
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
fn metadata_should_return_metadata_on_dir_if_exists(ctx: &'_ DistantServerCtx) {
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
fn metadata_should_return_metadata_on_symlink_if_exists(ctx: &'_ DistantServerCtx) {
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
fn metadata_should_include_canonicalized_path_if_flag_specified(ctx: &'_ DistantServerCtx) {
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
fn metadata_should_resolve_file_type_of_symlink_if_flag_specified(ctx: &'_ DistantServerCtx) {
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
