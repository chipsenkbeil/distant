//! E2E tests for the `distant connect` command with SSH.

use rstest::*;

use distant_test_harness::manager::{
    ManagerOnlyCtx, SshManagerCtx, manager_only_ctx, ssh_manager_ctx,
};

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
        .arg("ssh://127.0.0.1:1")
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
