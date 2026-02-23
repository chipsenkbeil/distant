use assert_fs::prelude::*;
use once_cell::sync::Lazy;

static TEMP_SCRIPT_DIR: Lazy<assert_fs::TempDir> = Lazy::new(|| assert_fs::TempDir::new().unwrap());

pub static SCRIPT_RUNNER: Lazy<String> =
    Lazy::new(|| String::from(if cfg!(windows) { "cmd.exe" } else { "bash" }));

pub static SCRIPT_RUNNER_ARG: Lazy<String> =
    Lazy::new(|| String::from(if cfg!(windows) { "/c" } else { "" }));

#[cfg(unix)]
pub static ECHO_ARGS_TO_STDOUT: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
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

#[cfg(windows)]
pub static ECHO_ARGS_TO_STDOUT: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
    let script = TEMP_SCRIPT_DIR.child("echo_args_to_stdout.cmd");
    script
        .write_str(indoc::indoc!(
            r#"
            @echo off
            echo %*
        "#
        ))
        .unwrap();
    script
});

#[cfg(unix)]
pub static ECHO_ARGS_TO_STDERR: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
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

#[cfg(windows)]
pub static ECHO_ARGS_TO_STDERR: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
    let script = TEMP_SCRIPT_DIR.child("echo_args_to_stderr.cmd");
    script
        .write_str(indoc::indoc!(
            r#"
            @echo off
            echo %* 1>&2
        "#
        ))
        .unwrap();
    script
});

#[cfg(unix)]
pub static ECHO_STDIN_TO_STDOUT: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
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

#[cfg(windows)]
pub static ECHO_STDIN_TO_STDOUT: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
    let script = TEMP_SCRIPT_DIR.child("echo_stdin_to_stdout.cmd");
    script
        .write_str(indoc::indoc!(
            r#"
            @echo off
            setlocal DisableDelayedExpansion

            set /p input=
            echo %input%
        "#
        ))
        .unwrap();
    script
});

#[cfg(unix)]
pub static EXIT_CODE: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
    let script = TEMP_SCRIPT_DIR.child("exit_code.sh");
    script
        .write_str(indoc::indoc!(
            r#"
            #!/usr/bin/env bash
            exit "$1"
        "#
        ))
        .unwrap();
    script
});

#[cfg(windows)]
pub static EXIT_CODE: Lazy<assert_fs::fixture::ChildPath> = Lazy::new(|| {
    let script = TEMP_SCRIPT_DIR.child("exit_code.cmd");
    script.write_str(r"EXIT /B %1").unwrap();
    script
});

pub static DOES_NOT_EXIST_BIN: Lazy<assert_fs::fixture::ChildPath> =
    Lazy::new(|| TEMP_SCRIPT_DIR.child("does_not_exist_bin"));
