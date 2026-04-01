//! Integration tests for `distant unmount`.

use std::process::{Command, Stdio};
use std::time::Duration;

use rstest::rstest;
use rstest_reuse::{self, *};

use distant_test_harness::backend::Backend;
use distant_test_harness::manager;
use distant_test_harness::mount::{self, MountBackend};
use distant_test_harness::skip_if_no_backend;

/// Spawns a foreground mount manually (not via `MountProcess`) and returns
/// the child process and canonicalized mount point path.
///
/// This avoids `MountProcess::drop` interference so the test can control
/// unmount timing.
fn spawn_raw_mount(
    ctx: &distant_test_harness::backend::BackendCtx,
    mount_backend: MountBackend,
    mount_point: &std::path::Path,
    args: &[&str],
) -> (std::process::Child, std::path::PathBuf) {
    std::fs::create_dir_all(mount_point).expect("failed to create mount point directory");

    let mut cmd = ctx.new_std_cmd(["mount"]);
    cmd.arg("--backend")
        .arg(mount_backend.as_str())
        .arg("--foreground");

    for arg in args {
        cmd.arg(arg);
    }

    cmd.arg(mount_point);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd.spawn().expect("failed to spawn distant mount process");

    let stdout = child.stdout.take().expect("stdout was not piped");
    let (tx, rx) = std::sync::mpsc::channel();

    std::thread::spawn(move || {
        use std::io::BufRead;
        let reader = std::io::BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if line.contains("Mounted") {
                let _ = tx.send(());
                return;
            }
        }
    });

    rx.recv_timeout(Duration::from_secs(30))
        .expect("mount process did not print 'Mounted' within 30s");

    let canonical = std::fs::canonicalize(mount_point).unwrap_or_else(|e| {
        panic!(
            "failed to canonicalize mount point {}: {e}",
            mount_point.display()
        )
    });

    (child, canonical)
}

/// UMT-01: Unmounting a specific mount point by path should succeed.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn unmount_by_path_should_succeed(#[case] backend: Backend, #[case] mount_backend: MountBackend) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-unmount-path");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "probe.txt"), "probe");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let (mut child, canonical) = spawn_raw_mount(
        &ctx,
        mount_backend,
        mount_dir.path(),
        &["--remote-root", &dir],
    );

    let output = Command::new(manager::bin_path())
        .args(["unmount", &canonical.to_string_lossy()])
        .output()
        .expect("failed to run unmount");

    assert!(
        output.status.success(),
        "[{backend:?}/{mount_backend}] unmount by path should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    mount::wait_for_unmount(&canonical);

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&canonical);
}

/// UMT-02: `unmount --all` should remove all active mounts.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn unmount_all_should_remove_everything(
    #[case] backend: Backend,
    #[case] mount_backend: MountBackend,
) {
    let ctx = skip_if_no_backend!(backend);

    let dir = ctx.unique_dir("mount-unmount-all");
    ctx.cli_mkdir(&dir);
    ctx.cli_write(&ctx.child_path(&dir, "probe.txt"), "probe");

    let mount_dir = assert_fs::TempDir::new().unwrap();
    let (mut child, canonical) = spawn_raw_mount(
        &ctx,
        mount_backend,
        mount_dir.path(),
        &["--remote-root", &dir],
    );

    let output = Command::new(manager::bin_path())
        .args(["unmount", "--all"])
        .output()
        .expect("failed to run unmount --all");

    assert!(
        output.status.success(),
        "[{backend:?}/{mount_backend}] unmount --all should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    mount::wait_for_unmount(&canonical);

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&canonical);
}

/// UMT-03: Unmounting a nonexistent path should fail with a non-zero exit code.
#[apply(super::plugin_x_mount)]
#[test_log::test]
fn unmount_nonexistent_should_fail(#[case] backend: Backend, #[case] mount_backend: MountBackend) {
    let _ = (backend, mount_backend);

    let bogus = "/tmp/distant-test-unmount-bogus-does-not-exist";

    let output = Command::new(manager::bin_path())
        .args(["unmount", bogus])
        .output()
        .expect("failed to run unmount");

    assert!(
        !output.status.success(),
        "unmounting a nonexistent path should fail"
    );
}
