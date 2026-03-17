//! E2E CLI tests for the SSH backend.
//!
//! Tests `distant connect ssh://` and `distant launch ssh://` workflows, plus
//! filesystem and process operations routed through the SSH plugin. Each test
//! spawns a real per-test sshd via the test harness.

use assert_fs::prelude::*;
use rstest::*;

use distant_test_harness::manager::*;

#[rstest]
#[test_log::test]
fn connect_ssh_establishes_connection(ssh_manager_ctx: SshManagerCtx) {
    let output = ssh_manager_ctx
        .new_std_cmd(["version"])
        .output()
        .expect("Failed to run version command");

    assert!(
        output.status.success(),
        "version should succeed after SSH connect, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.trim().is_empty(),
        "Expected version output, got empty stdout"
    );
}

#[rstest]
#[test_log::test]
fn connect_ssh_wrong_port(manager_only_ctx: ManagerOnlyCtx) {
    let output = manager_only_ctx
        .new_std_cmd(["connect"])
        .arg("ssh://localhost:1")
        .arg("--options")
        .arg("identities_only=true")
        .output()
        .expect("Failed to run connect command");

    assert!(
        !output.status.success(),
        "connect to SSH on port 1 should fail"
    );
}

#[rstest]
#[test_log::test]
fn connect_ssh_invalid_host(manager_only_ctx: ManagerOnlyCtx) {
    let output = manager_only_ctx
        .new_std_cmd(["connect"])
        .arg("ssh://nonexistent.invalid:22")
        .arg("--options")
        .arg("identities_only=true")
        .output()
        .expect("Failed to run connect command");

    assert!(
        !output.status.success(),
        "connect to nonexistent host should fail"
    );
}

#[rstest]
#[test_log::test]
fn launch_ssh_starts_remote_server(ssh_launch_ctx: SshLaunchCtx) {
    let output = ssh_launch_ctx
        .new_std_cmd(["version"])
        .output()
        .expect("Failed to run version command");

    assert!(
        output.status.success(),
        "version should succeed after SSH launch, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.trim().is_empty(),
        "Expected version output, got empty stdout"
    );
}

#[rstest]
#[test_log::test]
fn launch_ssh_wrong_credentials(manager_only_ctx: ManagerOnlyCtx) {
    let output = manager_only_ctx
        .new_std_cmd(["launch"])
        .arg("--distant")
        .arg(bin_path())
        .arg("ssh://localhost:22")
        .arg("--options")
        .arg("identity_files=/nonexistent/key,identities_only=true")
        .output()
        .expect("Failed to run launch command");

    assert!(
        !output.status.success(),
        "launch with bad credentials should fail"
    );
}

#[rstest]
#[test_log::test]
fn ssh_fs_read_file(ssh_manager_ctx: SshManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-read.txt");
    file.write_str("ssh read test content").unwrap();

    ssh_manager_ctx
        .new_assert_cmd(["fs", "read"])
        .arg(file.to_str().unwrap())
        .assert()
        .success()
        .stdout("ssh read test content");
}

#[rstest]
#[test_log::test]
fn ssh_fs_write_file(ssh_manager_ctx: SshManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("test-write.txt");

    ssh_manager_ctx
        .new_assert_cmd(["fs", "write"])
        .arg(file.to_str().unwrap())
        .write_stdin("ssh write test content")
        .assert()
        .success();

    // Give the OS time to flush to disk
    std::thread::sleep(std::time::Duration::from_millis(100));

    let contents = std::fs::read_to_string(file.path())
        .expect("Failed to read written file from local filesystem");
    assert_eq!(
        contents, "ssh write test content",
        "File contents should match what was written via SSH backend"
    );
}

#[rstest]
#[test_log::test]
fn ssh_spawn_process(ssh_manager_ctx: SshManagerCtx) {
    let output = ssh_manager_ctx
        .new_std_cmd(["spawn"])
        .args(["--", "echo", "hello-from-ssh"])
        .output()
        .expect("Failed to run spawn command");

    assert!(
        output.status.success(),
        "spawn via SSH should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("hello-from-ssh"),
        "Expected 'hello-from-ssh' in stdout, got: {stdout}"
    );
}

#[rstest]
#[test_log::test]
fn ssh_system_info(ssh_manager_ctx: SshManagerCtx) {
    let output = ssh_manager_ctx
        .new_std_cmd(["system-info"])
        .output()
        .expect("Failed to run system-info command");

    assert!(
        output.status.success(),
        "system-info via SSH should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Family:"),
        "Expected 'Family:' in system-info output, got: {stdout}"
    );
    assert!(
        stdout.contains("Arch:"),
        "Expected 'Arch:' in system-info output, got: {stdout}"
    );
}

#[rstest]
#[test_log::test]
fn ssh_fs_copy(ssh_manager_ctx: SshManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let src = temp.child("copy-src.txt");
    src.write_str("ssh copy content").unwrap();
    let dst = temp.child("copy-dst.txt");

    ssh_manager_ctx
        .new_assert_cmd(["fs", "copy"])
        .arg(src.to_str().unwrap())
        .arg(dst.to_str().unwrap())
        .assert()
        .success();

    let contents =
        std::fs::read_to_string(dst.path()).expect("Failed to read copied file from filesystem");
    assert_eq!(
        contents, "ssh copy content",
        "Copied file contents should match the source"
    );
}

#[rstest]
#[test_log::test]
fn ssh_fs_remove(ssh_manager_ctx: SshManagerCtx) {
    let temp = assert_fs::TempDir::new().unwrap();
    let file = temp.child("remove-me.txt");
    file.write_str("to be removed").unwrap();
    assert!(file.path().exists(), "file should exist before remove");

    ssh_manager_ctx
        .new_assert_cmd(["fs", "remove"])
        .arg(file.to_str().unwrap())
        .assert()
        .success();

    assert!(
        !file.path().exists(),
        "file should be gone after remove via SSH backend"
    );
}

#[rstest]
#[test_log::test]
fn ssh_shell_with_command(ssh_manager_ctx: SshManagerCtx) {
    let output = ssh_manager_ctx
        .new_std_cmd(["spawn"])
        .args(["--", "ls", "/tmp"])
        .output()
        .expect("Failed to run spawn -- ls /tmp via SSH");

    assert!(
        output.status.success(),
        "spawn -- ls /tmp via SSH should succeed, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[rstest]
#[test_log::test]
fn ssh_invalid_host(manager_only_ctx: ManagerOnlyCtx) {
    let output = manager_only_ctx
        .new_std_cmd(["connect"])
        .arg("ssh://nonexistent.invalid:22")
        .arg("--options")
        .arg("identities_only=true")
        .output()
        .expect("Failed to run connect command");

    assert!(
        !output.status.success(),
        "connect to nonexistent SSH host should fail"
    );
}
