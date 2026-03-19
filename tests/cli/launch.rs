//! E2E tests for the `distant launch` command with SSH.

use std::process::Stdio;
use std::time::Duration;

use rstest::*;

use distant_test_harness::manager::{
    self, ManagerOnlyCtx, SshLaunchCtx, manager_only_ctx, ssh_launch_ctx,
};

/// Maximum time to wait for a launch command to fail against an unresponsive
/// SSH server before concluding it is stuck on the handshake.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(15);

fn test_log_file(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("distant");
    std::fs::create_dir_all(&dir).ok();
    dir.join(format!("{name}.{}.log", rand::random::<u32>()))
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
        .arg(manager::bin_path())
        .arg("ssh://127.0.0.1:22")
        .arg("--options")
        .arg("identity_files=/nonexistent/key,identities_only=true")
        .output()
        .expect("Failed to run launch command");

    assert!(
        !output.status.success(),
        "launch with bad credentials should fail"
    );
}

#[tokio::test]
async fn launch_ssh_connection_timeout() {
    let ctx = ManagerOnlyCtx::start();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("failed to bind TCP listener");
    let port = listener.local_addr().unwrap().port();

    std::thread::spawn(move || {
        while let Ok((stream, _)) = listener.accept() {
            std::mem::forget(stream);
        }
    });

    let mut child = tokio::process::Command::new(manager::bin_path())
        .arg("launch")
        .arg("--distant")
        .arg(manager::bin_path())
        .arg(format!("ssh://127.0.0.1:{port}"))
        .arg("--options")
        .arg("identities_only=true")
        .arg("--log-file")
        .arg(test_log_file("client"))
        .arg("--log-level")
        .arg("trace")
        .arg(if cfg!(windows) {
            "--windows-pipe"
        } else {
            "--unix-socket"
        })
        .arg(ctx.socket_or_pipe())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("failed to spawn launch command");

    match tokio::time::timeout(LAUNCH_TIMEOUT, child.wait()).await {
        Ok(result) => {
            let status = result.expect("failed to wait on child");
            assert!(
                !status.success(),
                "launch to unresponsive SSH server should fail"
            );
        }
        Err(_) => {
            child.kill().await.ok();
        }
    }
}
