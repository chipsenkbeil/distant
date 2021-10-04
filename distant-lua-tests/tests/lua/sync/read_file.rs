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
            local status, _ = pcall(session.read_file_sync, session, { path = $file_path_str })
            assert(not status, "Unexpectedly succeeded!")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

#[rstest]
fn should_return_file_contents_as_byte_list(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("abcd").unwrap();
    let file_path = file.path();
    let file_path_str = file_path.to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local contents = session:read_file_sync({ path = $file_path_str })

            // abcd -> {97, 98, 99, 100}
            assert(type(contents) == "table", "Wrong content type: " .. type(contents))
            assert(contents[1] == 97, "Unexpected first byte: " .. contents[1])
            assert(contents[2] == 98, "Unexpected second byte: " .. contents[2])
            assert(contents[3] == 99, "Unexpected third byte: " .. contents[3])
            assert(contents[4] == 100, "Unexpected fourth byte: " .. contents[4])
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}
