//! E2E tests for the `distant connect` command.

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

/// Connects to a distant server using `distant://` credentials and verifies
/// the connection works by running a version command.
#[rstest]
#[test_log::test]
fn connect_distant_establishes_connection(manager_only_ctx: ManagerOnlyCtx) {
    let creds = manager_only_ctx.credentials.replace("0.0.0.0", "127.0.0.1");

    let connect_output = manager_only_ctx
        .new_std_cmd(["connect"])
        .arg(&creds)
        .output()
        .expect("Failed to run connect command");

    assert!(
        connect_output.status.success(),
        "connect with distant:// credentials should succeed, stderr: {}",
        String::from_utf8_lossy(&connect_output.stderr)
    );

    let version_output = manager_only_ctx
        .new_std_cmd(["version"])
        .output()
        .expect("Failed to run version command");

    assert!(
        version_output.status.success(),
        "version should succeed after distant connect, stderr: {}",
        String::from_utf8_lossy(&version_output.stderr)
    );

    let stdout = String::from_utf8_lossy(&version_output.stdout);
    assert!(
        !stdout.trim().is_empty(),
        "Expected version output, got empty stdout"
    );
}

/// Connects to a Docker container using `docker://` and verifies the
/// connection works by running a version command.
#[cfg(feature = "docker")]
#[test_log::test]
fn connect_docker_establishes_connection() {
    use distant_test_harness::docker;

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create runtime");

    let ctx = match rt.block_on(docker::DockerManagerCtx::start()) {
        Some(ctx) => ctx,
        None => {
            eprintln!("Docker not available — skipping test");
            return;
        }
    };

    let output = ctx
        .new_std_cmd(["version"])
        .output()
        .expect("Failed to run version command");

    assert!(
        output.status.success(),
        "version should succeed after Docker connect, stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.trim().is_empty(),
        "Expected version output, got empty stdout"
    );
}
