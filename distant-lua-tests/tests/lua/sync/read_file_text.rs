use crate::common::{fixtures::*, lua, session};
use assert_fs::prelude::*;
use mlua::chunk;
use rstest::*;

#[rstest]
fn should_return_error_if_fails_to_read_file(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("missing-file");
    let file_path = file.path();
    let file_path_str = file_path.to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local status, _ = pcall(session.read_file_text_sync, session, { path = $file_path_str })
            assert(not status, "Unexpectedly succeeded!")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

#[rstest]
fn should_return_file_contents_as_text(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("some file contents").unwrap();
    let file_path = file.path();
    let file_path_str = file_path.to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local contents = session:read_file_text_sync({ path = $file_path_str })
            assert(contents == "some file contents", "Unexpected file contents: " .. contents)
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}
