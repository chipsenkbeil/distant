//! E2E tests for the interactive `distant ssh` command.

use assert_fs::prelude::*;

use distant_test_harness::manager::{self, ManagerOnlyCtx};

fn test_log_file(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join("distant");
    std::fs::create_dir_all(&dir).ok();
    dir.join(format!("{name}.{}.log", rand::random::<u32>()))
}

#[tokio::test]
async fn ssh_shell_interactive() {
    use distant_test_harness::sshd;

    if which::which("sshd").is_err() {
        eprintln!("sshd not available -- skipping test");
        return;
    }

    let ctx = ManagerOnlyCtx::start();

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

    let pty_echo = distant_test_harness::exe::build_pty_echo()
        .await
        .expect("Failed to build pty-echo");
    let pty_echo_str = pty_echo.to_str().expect("pty-echo path is not valid UTF-8");

    let bin = manager::bin_path();
    let args = vec![
        "ssh".to_string(),
        format!("{}@127.0.0.1:{}", *sshd::USERNAME, port),
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

    let mut session = distant_test_harness::pty::PtySession::spawn(&bin, &args);

    session.send("abc");
    session.expect("abc");
}
