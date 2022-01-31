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

static ECHO_STDIN_TO_STDOUT_SH: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
    let script = TEMP_SCRIPT_DIR.child("echo_stdin_to_stdout.sh");
    script
        .write_str(indoc::indoc!(
            r#"
                #/usr/bin/env bash
                while IFS= read; do echo "$REPLY"; done
            "#
        ))
        .unwrap();
    script
});

static SLEEP_SH: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
    let script = TEMP_SCRIPT_DIR.child("sleep.sh");
    script
        .write_str(indoc::indoc!(
            r#"
                #!/usr/bin/env bash
                sleep "$1"
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
            local f = distant.utils.wrap_async(session.spawn_async, $schedule_fn)

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
fn should_return_back_process_on_success(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let cmd = SCRIPT_RUNNER.to_string();
    let args = vec![ECHO_ARGS_TO_STDOUT_SH.to_str().unwrap().to_string()];

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local distant = require("distant_lua")
            local f = distant.utils.wrap_async(session.spawn_async, $schedule_fn)

            // Because of our scheduler, the invocation turns async -> sync
            local err, proc
            f(session, { cmd = $cmd, args = $args }, function(success, res)
                if success then
                    proc = res
                else
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed to spawn process: " .. tostring(err))
            assert(proc.id >= 0, "Invalid process returned")
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

    let wait_fn = lua
        .create_function(|_, ()| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            Ok(())
        })
        .unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local distant = require("distant_lua")
            local f = distant.utils.wrap_async(session.spawn_async, $schedule_fn)

            // Because of our scheduler, the invocation turns async -> sync
            local err, proc
            f(session, { cmd = $cmd, args = $args }, function(success, res)
                if success then
                    proc = res
                else
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed to spawn process: " .. tostring(err))
            assert(proc, "Missing proc")

            // Wait briefly to ensure the process sends stdout
            $wait_fn()

            local f = distant.utils.wrap_async(proc.read_stdout_async, $schedule_fn)
            local err, stdout
            f(proc, function(success, res)
                if success then
                    stdout = res
                else
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed reading stdout: " .. tostring(err))

            stdout = string.char(unpack(stdout))
            assert(stdout == "some stdout", "Unexpected stdout: " .. stdout)
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

    let wait_fn = lua
        .create_function(|_, ()| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            Ok(())
        })
        .unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local distant = require("distant_lua")
            local f = distant.utils.wrap_async(session.spawn_async, $schedule_fn)

            // Because of our scheduler, the invocation turns async -> sync
            local err, proc
            f(session, { cmd = $cmd, args = $args }, function(success, res)
                if success then
                    proc = res
                else
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed to spawn process: " .. tostring(err))
            assert(proc, "Missing proc")

            // Wait briefly to ensure the process sends stdout
            $wait_fn()

            local f = distant.utils.wrap_async(proc.read_stderr_async, $schedule_fn)
            local err, stderr
            f(proc, function(success, res)
                if success then
                    stderr = res
                else
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed reading stdout: " .. tostring(err))

            stderr = string.char(unpack(stderr))
            assert(stderr == "some stderr", "Unexpected stderr: " .. stderr)
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

#[rstest]
fn should_return_error_when_killing_dead_process(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    // Spawn a process that will exit immediately, but is a valid process
    let cmd = SCRIPT_RUNNER.to_string();
    let args = vec![SLEEP_SH.to_str().unwrap().to_string(), String::from("0")];

    let wait_fn = lua
        .create_function(|_, ()| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            Ok(())
        })
        .unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local distant = require("distant_lua")
            local f = distant.utils.wrap_async(session.spawn_async, $schedule_fn)

            // Because of our scheduler, the invocation turns async -> sync
            local err, proc
            f(session, { cmd = $cmd, args = $args }, function(success, res)
                if success then
                    proc = res
                else
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed to spawn process: " .. tostring(err))
            assert(proc, "Missing proc")

            // Wait briefly to ensure the process dies
            $wait_fn()

            local f = distant.utils.wrap_async(proc.kill_async, $schedule_fn)
            local err
            f(proc, function(success, res)
                if not success then
                    err = res
                end
            end)
            assert(err, "Unexpectedly succeeded in killing dead process")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

#[rstest]
fn should_support_killing_processing(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let cmd = SCRIPT_RUNNER.to_string();
    let args = vec![SLEEP_SH.to_str().unwrap().to_string(), String::from("1")];

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local distant = require("distant_lua")
            local f = distant.utils.wrap_async(session.spawn_async, $schedule_fn)

            // Because of our scheduler, the invocation turns async -> sync
            local err, proc
            f(session, { cmd = $cmd, args = $args }, function(success, res)
                if success then
                    proc = res
                else
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed to spawn process: " .. tostring(err))
            assert(proc, "Missing proc")

            local f = distant.utils.wrap_async(proc.kill_async, $schedule_fn)
            local err
            f(proc, function(success, res)
                if not success then
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed to kill process: " .. tostring(err))
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

#[rstest]
fn should_return_error_if_sending_stdin_to_dead_process(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    // Spawn a process that will exit immediately, but is a valid process
    let cmd = SCRIPT_RUNNER.to_string();
    let args = vec![SLEEP_SH.to_str().unwrap().to_string(), String::from("0")];

    let wait_fn = lua
        .create_function(|_, ()| {
            std::thread::sleep(std::time::Duration::from_millis(50));
            Ok(())
        })
        .unwrap();

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local distant = require("distant_lua")
            local f = distant.utils.wrap_async(session.spawn_async, $schedule_fn)

            // Because of our scheduler, the invocation turns async -> sync
            local err, proc
            f(session, { cmd = $cmd, args = $args }, function(success, res)
                if success then
                    proc = res
                else
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed to spawn process: " .. tostring(err))
            assert(proc, "Missing proc")

            // Wait briefly to ensure the process dies
            $wait_fn()

            local f = distant.utils.wrap_async(proc.write_stdin_async, $schedule_fn)
            local err
            f(proc, "some text\n", function(success, res)
                if not success then
                    err = res
                end
            end)
            assert(err, "Unexpectedly succeeded")
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}

// NOTE: Ignoring on windows because it's using WSL which wants a Linux path
//       with / but thinks it's on windows and is providing \
#[rstest]
#[cfg_attr(windows, ignore)]
fn should_support_sending_stdin_to_spawned_process(ctx: &'_ DistantServerCtx) {
    let lua = lua::make().unwrap();
    let new_session = session::make_function(&lua, ctx).unwrap();
    let schedule_fn = poll::make_function(&lua).unwrap();

    let cmd = SCRIPT_RUNNER.to_string();
    let args = vec![ECHO_STDIN_TO_STDOUT_SH.to_str().unwrap().to_string()];

    let result = lua
        .load(chunk! {
            local session = $new_session()
            local distant = require("distant_lua")
            local f = distant.utils.wrap_async(session.spawn_async, $schedule_fn)

            // Because of our scheduler, the invocation turns async -> sync
            local err, proc
            f(session, { cmd = $cmd, args = $args }, function(success, res)
                if success then
                    proc = res
                else
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed spawning process: " .. tostring(err))
            assert(proc, "Missing proc")

            local f = distant.utils.wrap_async(proc.write_stdin_async, $schedule_fn)
            local err
            f(proc, "some text\n", function(success, res)
                if not success then
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed writing stdin: " .. tostring(err))

            local f = distant.utils.wrap_async(proc.read_stdout_async, $schedule_fn)
            local err, stdout
            f(proc, function(success, res)
                if success then
                    stdout = res
                else
                    err = res
                end
            end)
            assert(not err, "Unexpectedly failed reading stdout: " .. tostring(err))

            stdout = string.char(unpack(stdout))
            assert(stdout == "some text\n", "Unexpected stdout received: " .. stdout)
        })
        .exec();
    assert!(result.is_ok(), "Failed: {}", result.unwrap_err());
}
