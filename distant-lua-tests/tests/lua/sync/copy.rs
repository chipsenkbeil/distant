use crate::common::{fixtures::*, lua, session};
use assert_fs::prelude::*;
use mlua::chunk;
use predicates::prelude::*;
use rstest::*;

#[rstest]
fn should_send_error_on_failure(ctx: &'_ DistantServerCtx) {
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
fn should_support_copying_an_entire_directory(ctx: &'_ DistantServerCtx) {
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
fn should_support_copying_an_empty_directory(ctx: &'_ DistantServerCtx) {
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
fn should_support_copying_a_directory_that_only_contains_directories(ctx: &'_ DistantServerCtx) {
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
fn should_support_copying_a_single_file(ctx: &'_ DistantServerCtx) {
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
