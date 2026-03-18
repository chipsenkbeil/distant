//! Cross-plugin parity tests.
//!
//! Verifies that the same operations produce consistent results across
//! Host, SSH, and Docker backends. Uses [`BackendCtx`] to parameterize
//! each test over all available backends.
//!
//! All file operations use the `distant` CLI for both setup and verification
//! so that tests work regardless of whether the backend operates on the host
//! filesystem (Host, SSH) or inside a container (Docker).

use std::io::Write as _;
use std::path::PathBuf;
use std::process::Stdio;

use rstest::*;

use distant_test_harness::backend::{Backend, BackendCtx};
use distant_test_harness::skip_if_no_backend;

/// Returns a unique temp directory path valid for the backend's filesystem.
///
/// Docker containers always use `/tmp/` (Linux). Host and SSH use the
/// platform's temp directory (also `/tmp/` on Unix, `%TEMP%` on Windows).
fn unique_dir(ctx: &BackendCtx, label: &str) -> String {
    let id: u64 = rand::random();
    let base = match ctx.backend() {
        Backend::Docker => PathBuf::from("/tmp"),
        _ => std::env::temp_dir(),
    };
    base.join(format!("distant-parity-{label}-{id}"))
        .to_string_lossy()
        .to_string()
}

/// Joins a child filename to a parent directory, using the correct
/// path separator for the backend. Docker always uses `/` (Linux).
/// Host and SSH use the platform separator.
fn child_path(ctx: &BackendCtx, dir: &str, name: &str) -> String {
    match ctx.backend() {
        Backend::Docker => format!("{dir}/{name}"),
        _ => PathBuf::from(dir).join(name).to_string_lossy().to_string(),
    }
}

/// Creates a file through the distant CLI, works for all backends.
fn cli_write(ctx: &BackendCtx, path: &str, content: &str) {
    let mut child = ctx
        .new_std_cmd(["fs", "write"])
        .arg(path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn fs write");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(content.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "fs write setup failed for {path}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Reads a file through the distant CLI, works for all backends.
fn cli_read(ctx: &BackendCtx, path: &str) -> String {
    let output = ctx
        .new_std_cmd(["fs", "read"])
        .arg(path)
        .output()
        .expect("failed to run fs read");
    assert!(
        output.status.success(),
        "fs read failed for {path}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// Checks if a path exists through the distant CLI, works for all backends.
fn cli_exists(ctx: &BackendCtx, path: &str) -> bool {
    let output = ctx
        .new_std_cmd(["fs", "exists"])
        .arg(path)
        .output()
        .expect("failed to run fs exists");
    output.status.success() && String::from_utf8_lossy(&output.stdout).contains("true")
}

/// Creates a directory through the distant CLI, works for all backends.
fn cli_mkdir(ctx: &BackendCtx, path: &str) {
    let output = ctx
        .new_std_cmd(["fs", "make-dir"])
        .arg(path)
        .output()
        .expect("failed to run fs make-dir");
    assert!(
        output.status.success(),
        "fs make-dir failed for {path}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn fs_read_file(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = unique_dir(&ctx, "read");
    cli_mkdir(&ctx, &dir);
    let path = child_path(&ctx, &dir, "read-test.txt");
    cli_write(&ctx, &path, "parity read content");

    ctx.new_assert_cmd(["fs", "read"])
        .arg(&path)
        .assert()
        .success()
        .stdout("parity read content");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn fs_write_file(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = unique_dir(&ctx, "write");
    cli_mkdir(&ctx, &dir);
    let path = child_path(&ctx, &dir, "write-test.txt");

    // This is the operation under test
    let mut child = ctx
        .new_std_cmd(["fs", "write"])
        .arg(&path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn fs write");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"parity write content")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "fs write should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify via CLI read (works for all backends)
    let contents = cli_read(&ctx, &path);
    assert_eq!(contents, "parity write content");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn fs_copy(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = unique_dir(&ctx, "copy");
    cli_mkdir(&ctx, &dir);
    let src = child_path(&ctx, &dir, "copy-src.txt");
    let dst = child_path(&ctx, &dir, "copy-dst.txt");
    cli_write(&ctx, &src, "parity copy content");

    ctx.new_assert_cmd(["fs", "copy"])
        .arg(&src)
        .arg(&dst)
        .assert()
        .success();

    let contents = cli_read(&ctx, &dst);
    assert_eq!(contents, "parity copy content");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn fs_exists(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = unique_dir(&ctx, "exists");
    cli_mkdir(&ctx, &dir);
    let path = child_path(&ctx, &dir, "exists-test.txt");
    cli_write(&ctx, &path, "exists");

    let output = ctx
        .new_std_cmd(["fs", "exists"])
        .arg(&path)
        .output()
        .expect("Failed to run fs exists");

    assert!(
        output.status.success(),
        "fs exists should succeed for existing file, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("true"),
        "Expected 'true' for existing file, got: {stdout}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn fs_make_dir(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = unique_dir(&ctx, "mkdir");
    cli_mkdir(&ctx, &dir);
    let new_dir = child_path(&ctx, &dir, "new-dir");

    ctx.new_assert_cmd(["fs", "make-dir"])
        .arg(&new_dir)
        .assert()
        .success();

    assert!(
        cli_exists(&ctx, &new_dir),
        "Directory should be created (verified via CLI)"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn fs_remove(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = unique_dir(&ctx, "remove");
    cli_mkdir(&ctx, &dir);
    let path = child_path(&ctx, &dir, "remove-test.txt");
    cli_write(&ctx, &path, "to be removed");
    assert!(cli_exists(&ctx, &path), "File should exist before removal");

    ctx.new_assert_cmd(["fs", "remove"])
        .arg(&path)
        .assert()
        .success();

    assert!(
        !cli_exists(&ctx, &path),
        "File should be removed (verified via CLI)"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn fs_rename(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = unique_dir(&ctx, "rename");
    cli_mkdir(&ctx, &dir);
    let src = child_path(&ctx, &dir, "rename-src.txt");
    let dst = child_path(&ctx, &dir, "rename-dst.txt");
    cli_write(&ctx, &src, "rename content");

    ctx.new_assert_cmd(["fs", "rename"])
        .arg(&src)
        .arg(&dst)
        .assert()
        .success();

    assert!(
        !cli_exists(&ctx, &src),
        "Source should no longer exist (verified via CLI)"
    );
    assert!(
        cli_exists(&ctx, &dst),
        "Destination should exist (verified via CLI)"
    );
    let contents = cli_read(&ctx, &dst);
    assert_eq!(contents, "rename content");
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn fs_metadata(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = unique_dir(&ctx, "metadata");
    cli_mkdir(&ctx, &dir);
    let path = child_path(&ctx, &dir, "metadata-test.txt");
    cli_write(&ctx, &path, "metadata content");

    let output = ctx
        .new_std_cmd(["fs", "metadata"])
        .arg(&path)
        .output()
        .expect("Failed to run fs metadata");

    assert!(
        output.status.success(),
        "fs metadata should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Type:") || stdout.contains("type"),
        "Expected metadata output containing type info, got: {stdout}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn spawn(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let output = ctx
        .new_std_cmd(["spawn"])
        .args(["--", "echo", "parity-spawn-ok"])
        .output()
        .expect("Failed to run spawn");

    assert!(
        output.status.success(),
        "spawn should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("parity-spawn-ok"),
        "Expected 'parity-spawn-ok' in stdout, got: {stdout}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn system_info(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let output = ctx
        .new_std_cmd(["system-info"])
        .output()
        .expect("Failed to run system-info");

    assert!(
        output.status.success(),
        "system-info should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Family:"),
        "Expected 'Family:' in output, got: {stdout}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn version(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let output = ctx
        .new_std_cmd(["version"])
        .output()
        .expect("Failed to run version");

    assert!(
        output.status.success(),
        "version should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.trim().is_empty(),
        "Expected version output, got empty"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn fs_read_dir(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = unique_dir(&ctx, "readdir");
    cli_mkdir(&ctx, &dir);
    cli_write(&ctx, &child_path(&ctx, &dir, "aaa.txt"), "a");
    cli_write(&ctx, &child_path(&ctx, &dir, "bbb.txt"), "b");

    // `distant fs read <dir>` returns directory entries when given a directory
    let output = ctx
        .new_std_cmd(["fs", "read"])
        .arg(&dir)
        .output()
        .expect("Failed to run fs read (directory)");

    assert!(
        output.status.success(),
        "fs read (directory) should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("aaa.txt"),
        "Expected 'aaa.txt' in directory listing, got: {stdout}"
    );
    assert!(
        stdout.contains("bbb.txt"),
        "Expected 'bbb.txt' in directory listing, got: {stdout}"
    );
}

#[rstest]
#[case::host(Backend::Host)]
#[case::ssh(Backend::Ssh)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn fs_set_permissions(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = unique_dir(&ctx, "perms");
    cli_mkdir(&ctx, &dir);
    let path = child_path(&ctx, &dir, "perms-test.txt");
    cli_write(&ctx, &path, "perms content");

    // Set file to readonly using chmod-style mode
    ctx.new_assert_cmd(["fs", "set-permissions"])
        .arg("readonly")
        .arg(&path)
        .assert()
        .success();

    // Verify the file is still readable
    ctx.new_assert_cmd(["fs", "read"])
        .arg(&path)
        .assert()
        .success()
        .stdout("perms content");
}

/// Search is not supported over the SSH plugin (returns an error).
/// Only Host and Docker backends are tested.
#[rstest]
#[case::host(Backend::Host)]
#[case::docker(Backend::Docker)]
#[test_log::test]
fn fs_search(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);
    let dir = unique_dir(&ctx, "search");
    cli_mkdir(&ctx, &dir);
    cli_write(
        &ctx,
        &child_path(&ctx, &dir, "needle.txt"),
        "haystack needle haystack",
    );
    cli_write(&ctx, &child_path(&ctx, &dir, "other.txt"), "no match here");

    // `distant fs search <pattern> [PATHS]...` — pattern is the first positional arg
    let output = ctx
        .new_std_cmd(["fs", "search"])
        .arg("needle")
        .arg(&dir)
        .output()
        .expect("Failed to run fs search");

    assert!(
        output.status.success(),
        "fs search should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("needle.txt"),
        "Expected 'needle.txt' in search results, got: {stdout}"
    );
}

/// Watch is only tested on Host backend because it requires host-level
/// filesystem events (inotify/FSEvents/ReadDirectoryChanges). Docker
/// containers use overlayfs which may not propagate inotify events
/// reliably. SSH-connected backends also use the host filesystem but
/// adding watch tests for SSH would be redundant given that the same
/// code path is exercised.
#[rstest]
#[case::host(Backend::Host)]
#[test_log::test]
fn fs_watch(#[case] backend: Backend) {
    use std::time::Duration;

    use assert_fs::prelude::*;

    let ctx = skip_if_no_backend!(backend);
    let temp = assert_fs::TempDir::new().unwrap();

    // Start watching the temp directory for create events.
    // `distant fs watch` is a streaming command — it runs until killed.
    let mut child = ctx
        .new_std_cmd(["fs", "watch"])
        .arg(temp.to_str().unwrap())
        .arg("--recursive")
        .arg("--only")
        .arg("create")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn fs watch");

    // Give the watch time to establish
    std::thread::sleep(Duration::from_secs(1));

    // Create a file in the watched directory — this should trigger an event
    temp.child("watched-file.txt")
        .write_str("watch content")
        .unwrap();

    // Give the watch event time to propagate and be written to stdout
    std::thread::sleep(Duration::from_secs(2));

    // Kill the watch process and collect output
    child.kill().expect("Failed to kill watch process");
    let output = child
        .wait_with_output()
        .expect("Failed to wait for watch process");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("watched-file.txt"),
        "Expected 'watched-file.txt' in watch output, got: {stdout}"
    );
}

/// Verifies that `distant spawn --pty` works by running a simple echo command
/// through a PTY-allocated remote process. Uses `PtySession` (portable-pty)
/// because `--pty` requires a real terminal (raw mode). Only tested on Host
/// backend; SSH and Docker PTY tests live in their respective modules.
#[rstest]
#[case::host(Backend::Host)]
#[tokio::test]
async fn spawn_with_pty(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(backend);

    let (bin, mut args) = match &ctx {
        BackendCtx::Host(c) => c.cmd_parts(["spawn"]),
        _ => unreachable!("spawn_with_pty only tests Host backend"),
    };

    args.push("--pty".to_string());
    args.push("--".to_string());

    // On Windows, `echo` is a cmd.exe built-in (no echo.exe).
    #[cfg(windows)]
    {
        args.push("cmd".to_string());
        args.push("/c".to_string());
    }
    args.push("echo".to_string());
    args.push("pty-spawn-parity".to_string());

    let mut session = super::pty::PtySession::spawn(&bin, &args);
    session.expect("pty-spawn-parity");
}

/// Tests `distant kill` by killing the active connection and verifying that
/// subsequent commands fail. Uses HostManagerCtx directly (not BackendCtx)
/// because we need the connection to be killable and then verify failure.
#[test_log::test]
fn kill_connection() {
    use distant_test_harness::manager;

    let ctx = manager::HostManagerCtx::start();

    // First verify the connection works
    let output = ctx
        .new_std_cmd(["version"])
        .output()
        .expect("Failed to run version");
    assert!(
        output.status.success(),
        "version should succeed before kill"
    );

    // Get the connection ID from `distant status`
    let status_output = ctx
        .new_std_cmd(["status"])
        .output()
        .expect("Failed to run status");

    // `distant status` outputs connection info to stderr in the format:
    //   * <connection_id> -> distant://...
    let status_stdout = String::from_utf8_lossy(&status_output.stdout);
    let status_stderr = String::from_utf8_lossy(&status_output.stderr);

    // Search both stdout and stderr since output destination may vary.
    // Connection lines have the format:
    //   * <id> -> distant://...   (selected connection)
    //     <id> -> distant://...   (other connections)
    let combined = format!("{status_stdout}\n{status_stderr}");
    let conn_id = combined
        .lines()
        .find_map(|line| {
            let trimmed = line.trim().strip_prefix("* ").unwrap_or(line.trim());
            // A connection line contains " -> " separator
            if !trimmed.contains(" -> ") {
                return None;
            }
            trimmed.split_whitespace().next()
        })
        .unwrap_or_else(|| {
            panic!(
                "Failed to find connection ID in status output.\nstdout: {status_stdout}\nstderr: {status_stderr}"
            )
        });

    // Kill the connection
    ctx.new_assert_cmd(["kill"]).arg(conn_id).assert().success();

    // After killing, commands should fail
    let output = ctx
        .new_std_cmd(["version"])
        .output()
        .expect("Failed to run version after kill");

    assert!(
        !output.status.success(),
        "version should fail after connection is killed"
    );
}
