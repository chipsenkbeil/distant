use crate::common::{fixtures::*, lua, poll, session};
use assert_fs::prelude::*;
use mlua::chunk;
use once_cell::sync::Lazy;
use rstest::*;

static TEMP_SCRIPT_DIR: Lazy<assert_fs::TempDir> = Lazy::new(|| assert_fs::TempDir::new().unwrap());
static SCRIPT_RUNNER: Lazy<String> = Lazy::new(|| String::from("bash"));

static ECHO_ARGS_TO_STDOUT_SH: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
    let script = TEMP_SCRIPT_DIR.child("echo_args_to_stdout.sh");
    script
        .write_str(indoc::indoc!(
            r#"
                #/usr/bin/env bash
                printf "%s" "$*"
            "#
        ))
        .unwrap();
    script
});

static ECHO_ARGS_TO_STDERR_SH: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
    let script = TEMP_SCRIPT_DIR.child("echo_args_to_stderr.sh");
    script
        .write_str(indoc::indoc!(
            r#"
                #/usr/bin/env bash
                printf "%s" "$*" 1>&2
            "#
        ))
        .unwrap();
    script
});

static DOES_NOT_EXIST_BIN: Lazy<assert_fs::fixture::ChildPath> =
    Lazy::new(|| TEMP_SCRIPT_DIR.child("does_not_exist_bin"));

#[rstest]
fn should_return_error_on_failure(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let cmd = DOES_NOT_EXIST_BIN.to_str().unwrap().to_string();
    let args: Vec<String> = Vec::new();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local distant = require("distant_lua")
            local f = distant.utils.wrap_async(session.spawn_wait_async, $schedule_fn)

            // Because of our scheduler, the invocation turns async -> sync
            local err
            f(session, { cmd = $cmd, args = $args }, function(success, res)
                if not success then
                    err = res
                end
            end)
            assert(err, "Unexpectedly succeeded")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

#[rstest]
fn should_return_back_status_on_success(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let cmd = SCRIPT_RUNNER.to_string();
    let args = vec![ECHO_ARGS_TO_STDOUT_SH.to_str().unwrap().to_string()];

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local distant = require("distant_lua")
            local f = distant.utils.wrap_async(session.spawn_wait_async, $schedule_fn)

            // Because of our scheduler, the invocation turns async -> sync
            local err, output
            f(session, { cmd = $cmd, args = $args }, function(success, res)
                if success then
                    output = res
                else
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed to spawn process: " .. tostring(err))
            assert(output, "Missing process output")
            assert(output.success, "Process output returned !success")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

// NOTE: Ignoring on windows because it's using WSL which wants a Linux path
//       with / but thinks it's on windows and is providing \
#[rstest]
#[cfg_attr(windows, ignore)]
fn should_return_process_that_can_retrieve_stdout(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let cmd = SCRIPT_RUNNER.to_string();
    let args = vec![
        ECHO_ARGS_TO_STDOUT_SH.to_str().unwrap().to_string(),
        String::from("some stdout"),
    ];

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local distant = require("distant_lua")
            local f = distant.utils.wrap_async(session.spawn_wait_async, $schedule_fn)

            // Because of our scheduler, the invocation turns async -> sync
            local err, output
            f(session, { cmd = $cmd, args = $args }, function(success, res)
                if success then
                    output = res
                else
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed to spawn process: " .. tostring(err))
            assert(output, "Missing process output")
            assert(output.stdout, "some stdout")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

// NOTE: Ignoring on windows because it's using WSL which wants a Linux path
//       with / but thinks it's on windows and is providing \
#[rstest]
#[cfg_attr(windows, ignore)]
fn should_return_process_that_can_retrieve_stderr(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let cmd = SCRIPT_RUNNER.to_string();
    let args = vec![
        ECHO_ARGS_TO_STDERR_SH.to_str().unwrap().to_string(),
        String::from("some stderr"),
    ];

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local distant = require("distant_lua")
            local f = distant.utils.wrap_async(session.spawn_wait_async, $schedule_fn)

            // Because of our scheduler, the invocation turns async -> sync
            local err, output
            f(session, { cmd = $cmd, args = $args }, function(success, res)
                if success then
                    output = res
                else
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed to spawn process: " .. tostring(err))
            assert(output, "Missing process output")
            assert(output.stderr, "some stderr")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}
