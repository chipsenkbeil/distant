use crate::common::{fixtures::*, lua, session};
use assert_fs::prelude::*;
use mlua::chunk;
use rstest::*;

#[rstest]
fn should_send_true_if_path_exists(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    file.touch().unwrap();
    let file_path = file.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local exists = session:exists({ path = $file_path })
            assert(exists, "File unexpectedly missing")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

#[rstest]
fn should_send_false_if_path_does_not_exist(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("file");
    let file_path = file.path().to_str().unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local exists = session:exists({ path = $file_path })
            assert(not exists, "File unexpectedly found")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}
