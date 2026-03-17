//! Cross-plugin parity tests.
//!
//! Verifies that the same operations produce consistent results across
//! Host, SSH, and Docker backends. Uses [`BackendCtx`] to parameterize
//! each test over all available backends.

use assert_fs::prelude::*;
use rstest::*;

use distant_test_harness::backend::{Backend, ctx_for_backend};
use distant_test_harness::skip_if_no_backend;

// ---------------------------------------------------------------------------
// Filesystem operations
// ---------------------------------------------------------------------------

#[rstest]
#[case(Backend::Host)]
#[case(Backend::Ssh)]
#[case(Backend::Docker)]
#[test_log::test]
fn fs_read_file(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(ctx_for_backend(backend));
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("read-test.txt");
    file.write_str("parity read content").unwrap();

    ctx.new_assert_cmd(["fs", "read"])
        .arg(file.to_str().unwrap())
        .assert()
        .success()
        .stdout("parity read content");
}

#[rstest]
#[case(Backend::Host)]
#[case(Backend::Ssh)]
#[case(Backend::Docker)]
#[test_log::test]
fn fs_write_file(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(ctx_for_backend(backend));
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("write-test.txt");

    ctx.new_assert_cmd(["fs", "write"])
        .arg(file.to_str().unwrap())
        .write_stdin("parity write content")
        .assert()
        .success();

    std::thread::sleep(std::time::Duration::from_millis(100));
    let contents =
        std::fs::read_to_string(file.path()).expect("Failed to read written file from disk");
    assert_eq!(contents, "parity write content");
}

#[rstest]
#[case(Backend::Host)]
#[case(Backend::Ssh)]
#[case(Backend::Docker)]
#[test_log::test]
fn fs_copy(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(ctx_for_backend(backend));
    let temp = assert_fs::TempDir::new().unwrap();
    let src = temp.child("copy-src.txt");
    src.write_str("parity copy content").unwrap();
    let dst = temp.child("copy-dst.txt");

    ctx.new_assert_cmd(["fs", "copy"])
        .arg(src.to_str().unwrap())
        .arg(dst.to_str().unwrap())
        .assert()
        .success();

    let contents =
        std::fs::read_to_string(dst.path()).expect("Failed to read copied file from disk");
    assert_eq!(contents, "parity copy content");
}

#[rstest]
#[case(Backend::Host)]
#[case(Backend::Ssh)]
#[case(Backend::Docker)]
#[test_log::test]
fn fs_exists(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(ctx_for_backend(backend));
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("exists-test.txt");
    file.write_str("exists").unwrap();

    let output = ctx
        .new_std_cmd(["fs", "exists"])
        .arg(file.to_str().unwrap())
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
#[case(Backend::Host)]
#[case(Backend::Ssh)]
#[case(Backend::Docker)]
#[test_log::test]
fn fs_make_dir(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(ctx_for_backend(backend));
    let temp = assert_fs::TempDir::new().unwrap();
    let dir = temp.child("new-dir");

    ctx.new_assert_cmd(["fs", "make-dir"])
        .arg(dir.to_str().unwrap())
        .assert()
        .success();

    assert!(dir.path().is_dir(), "Directory should be created");
}

#[rstest]
#[case(Backend::Host)]
#[case(Backend::Ssh)]
#[case(Backend::Docker)]
#[test_log::test]
fn fs_remove(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(ctx_for_backend(backend));
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("remove-test.txt");
    file.write_str("to be removed").unwrap();
    assert!(file.path().exists());

    ctx.new_assert_cmd(["fs", "remove"])
        .arg(file.to_str().unwrap())
        .assert()
        .success();

    assert!(!file.path().exists(), "File should be removed");
}

#[rstest]
#[case(Backend::Host)]
#[case(Backend::Ssh)]
#[case(Backend::Docker)]
#[test_log::test]
fn fs_rename(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(ctx_for_backend(backend));
    let temp = assert_fs::TempDir::new().unwrap();
    let src = temp.child("rename-src.txt");
    src.write_str("rename content").unwrap();
    let dst = temp.child("rename-dst.txt");

    ctx.new_assert_cmd(["fs", "rename"])
        .arg(src.to_str().unwrap())
        .arg(dst.to_str().unwrap())
        .assert()
        .success();

    assert!(!src.path().exists(), "Source should no longer exist");
    assert!(dst.path().exists(), "Destination should exist");
    let contents = std::fs::read_to_string(dst.path()).unwrap();
    assert_eq!(contents, "rename content");
}

#[rstest]
#[case(Backend::Host)]
#[case(Backend::Ssh)]
#[case(Backend::Docker)]
#[test_log::test]
fn fs_metadata(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(ctx_for_backend(backend));
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("metadata-test.txt");
    file.write_str("metadata content").unwrap();

    let output = ctx
        .new_std_cmd(["fs", "metadata"])
        .arg(file.to_str().unwrap())
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

// ---------------------------------------------------------------------------
// Process operations
// ---------------------------------------------------------------------------

#[rstest]
#[case(Backend::Host)]
#[case(Backend::Ssh)]
#[case(Backend::Docker)]
#[test_log::test]
fn spawn(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(ctx_for_backend(backend));

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

// ---------------------------------------------------------------------------
// System operations
// ---------------------------------------------------------------------------

#[rstest]
#[case(Backend::Host)]
#[case(Backend::Ssh)]
#[case(Backend::Docker)]
#[test_log::test]
fn system_info(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(ctx_for_backend(backend));

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
#[case(Backend::Host)]
#[case(Backend::Ssh)]
#[case(Backend::Docker)]
#[test_log::test]
fn version(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(ctx_for_backend(backend));

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

// ---------------------------------------------------------------------------
// Additional filesystem operations
// ---------------------------------------------------------------------------

#[rstest]
#[case(Backend::Host)]
#[case(Backend::Ssh)]
#[case(Backend::Docker)]
#[test_log::test]
fn fs_read_dir(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(ctx_for_backend(backend));
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("aaa.txt").write_str("a").unwrap();
    temp.child("bbb.txt").write_str("b").unwrap();

    // `distant fs read <dir>` returns directory entries when given a directory
    let output = ctx
        .new_std_cmd(["fs", "read"])
        .arg(temp.to_str().unwrap())
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
#[case(Backend::Host)]
#[case(Backend::Ssh)]
#[case(Backend::Docker)]
#[test_log::test]
fn fs_set_permissions(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(ctx_for_backend(backend));
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("perms-test.txt");
    file.write_str("perms content").unwrap();

    // Set file to readonly using chmod-style mode
    ctx.new_assert_cmd(["fs", "set-permissions"])
        .arg("readonly")
        .arg(file.to_str().unwrap())
        .assert()
        .success();

    // Verify the file is still readable
    ctx.new_assert_cmd(["fs", "read"])
        .arg(file.to_str().unwrap())
        .assert()
        .success()
        .stdout("perms content");
}

/// Search is not supported over the SSH plugin (returns an error).
/// Only Host and Docker backends are tested.
#[rstest]
#[case(Backend::Host)]
#[case(Backend::Docker)]
#[test_log::test]
fn fs_search(#[case] backend: Backend) {
    let ctx = skip_if_no_backend!(ctx_for_backend(backend));
    let temp = assert_fs::TempDir::new().unwrap();
    temp.child("needle.txt")
        .write_str("haystack needle haystack")
        .unwrap();
    temp.child("other.txt").write_str("no match here").unwrap();

    // `distant fs search <pattern> [PATHS]...` — pattern is the first positional arg
    let output = ctx
        .new_std_cmd(["fs", "search"])
        .arg("needle")
        .arg(temp.to_str().unwrap())
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

// Note: `kill` (proc_kill) is tested via the JSON API tests in `tests/cli/api/`.
// Cross-backend kill testing would require API-level interaction that is
// already covered by the existing proc_spawn/proc_kill API test suite.
