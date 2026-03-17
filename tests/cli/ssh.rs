//! E2E CLI tests for the SSH backend.
//!
//! Tests `distant connect ssh://` and `distant launch ssh://` workflows, plus
//! filesystem and process operations routed through the SSH plugin. Each test
//! spawns a real per-test sshd via the test harness.

use std::process::Stdio;
use std::time::Duration;

use assert_fs::prelude::*;
use rstest::*;

use distant_test_harness::manager::*;

fn test_log_file(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("distant");
    std::fs::create_dir_all(&dir).ok();
    dir.join(format!("{name}.{}.log", rand::random::<u32>()))
}

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

#[tokio::test]
async fn launch_ssh_connection_timeout() {
    let ctx = ManagerOnlyCtx::start();

    // Create a TCP listener that accepts connections but never responds,
    // simulating an SSH server that hangs during the handshake.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("failed to bind TCP listener");
    let port = listener.local_addr().unwrap().port();

    // Accept connections in a background thread but never send data.
    // The accepted stream is leaked so the TCP connection stays open
    // (the SSH client sees the SYN-ACK but never gets a version banner).
    std::thread::spawn(move || {
        while let Ok((stream, _)) = listener.accept() {
            std::mem::forget(stream);
        }
    });

    // Run `distant launch` against the unresponsive "SSH server".
    // The SSH handshake will never begin because the server never sends data.
    let mut child = tokio::process::Command::new(bin_path())
        .arg("launch")
        .arg("--distant")
        .arg(bin_path())
        .arg(format!("ssh://localhost:{port}"))
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

    // The SSH handshake should fail or the process should hang. Either way,
    // the launch must NOT succeed. We give it 15 seconds to fail on its own
    // (russh has internal timeouts); if it hasn't exited by then, we kill it.
    match tokio::time::timeout(Duration::from_secs(15), child.wait()).await {
        Ok(result) => {
            let status = result.expect("failed to wait on child");
            assert!(
                !status.success(),
                "launch to unresponsive SSH server should fail"
            );
        }
        Err(_) => {
            // The process is still running after 15s — it's stuck on
            // the SSH handshake, which confirms the timeout scenario.
            child.kill().await.ok();
        }
    }
}

#[tokio::test]
async fn ssh_shell_interactive() {
    use distant_test_harness::sshd;

    if which::which("sshd").is_err() {
        eprintln!("sshd not available — skipping test");
        return;
    }

    let ctx = ManagerOnlyCtx::start();

    // Spawn a per-test sshd
    let sshd = sshd::sshd();
    let port = sshd.port;
    let identity_file = sshd
        .tmp
        .child("id_ed25519")
        .path()
        .to_string_lossy()
        .to_string();
    let known_hosts = sshd
        .tmp
        .child("known_hosts")
        .path()
        .to_string_lossy()
        .to_string();

    let options = format!(
        "identity_files={},user_known_hosts_files={},identities_only=true",
        identity_file, known_hosts,
    );

    // Build `distant ssh` command to connect to the test sshd and run the
    // pty-echo helper binary. `distant ssh` auto-connects via the manager,
    // creating an SSH connection and opening a PTY session.
    //
    // This tests the full interactive I/O path through `distant ssh` (which
    // is separate from `distant shell` — it handles the SSH connect flow
    // itself). The pty-echo binary echoes bytes back through the PTY,
    // proving bidirectional interactive communication works.
    let pty_echo = distant_test_harness::exe::build_pty_echo()
        .await
        .expect("Failed to build pty-echo");
    let pty_echo_str = pty_echo.to_str().expect("pty-echo path is not valid UTF-8");

    let bin = bin_path();
    let args = vec![
        "ssh".to_string(),
        format!("{}@localhost:{}", *sshd::USERNAME, port),
        "--options".to_string(),
        options,
        "--predict".to_string(),
        "off".to_string(),
        "--log-file".to_string(),
        test_log_file("client").to_string_lossy().to_string(),
        "--log-level".to_string(),
        "trace".to_string(),
        if cfg!(windows) {
            "--windows-pipe".to_string()
        } else {
            "--unix-socket".to_string()
        },
        ctx.socket_or_pipe().to_string(),
        "--".to_string(),
        pty_echo_str.to_string(),
    ];

    let mut session = super::pty::PtySession::spawn(&bin, &args);

    // Send text and verify it's echoed back through the full
    // distant ssh -> SSH connection -> remote pty-echo -> SSH -> distant ssh path
    session.send("abc");
    session.expect("abc");
}
