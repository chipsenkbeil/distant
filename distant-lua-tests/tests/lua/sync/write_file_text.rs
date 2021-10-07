use crate::common::{fixtures::*, lua, session};
use assert_fs::prelude::*;
use mlua::chunk;
use predicates::prelude::*;
use rstest::*;

#[rstest]
fn should_yield_error_if_fails_to_create_file(ctx: &'_ DistantServerCtx) {
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
            local status, _ = pcall(session.write_file_text, session, {
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
fn should_overwrite_existing_file(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();

    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-file");
    file.write_str("line 1").unwrap();

    let file_path = file.path().to_str().unwrap();
    let text = "some text".to_string();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            session:write_file_text({
                path = $file_path,
                data = $text
            })
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());

    // Also verify that we appended to the file
    file.assert("some text");
}
