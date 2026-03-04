//! Process management utilities for test cleanup.
//!
//! Provides cross-platform helpers to spawn processes in their own process
//! group and kill entire process trees, preventing leaked handles detected
//! by nextest.

use std::process::Child;

/// Configures a command to spawn in its own process group.
///
/// On Unix, this calls `process_group(0)` so the child becomes the leader of a
/// new process group. Combined with [`kill_process_tree`], this ensures all
/// descendants are killed during test cleanup.
///
/// On Windows this is a no-op — [`kill_process_tree`] uses `taskkill /T` which
/// handles tree killing natively.
#[cfg(unix)]
pub fn set_process_group(cmd: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
}

/// No-op on non-Unix platforms.
#[cfg(not(unix))]
#[allow(clippy::needless_pass_by_ref_mut)]
pub fn set_process_group(_cmd: &mut std::process::Command) {}

/// Recursively collects all descendant PIDs of the given process.
///
/// Uses `pgrep -P` to find children, then recurses into each child. PIDs are
/// pushed in post-order (deepest descendants first) so that callers can kill
/// them leaf-to-root.
///
/// This catches descendants that called `setsid()` and left the process group
/// (e.g. `sshd-session` with PTY allocation, daemonized servers).
#[cfg(unix)]
fn collect_descendants(pid: u32, pids: &mut Vec<u32>) {
    if let Ok(output) = std::process::Command::new("pgrep")
        .args(["-P", &pid.to_string()])
        .output()
        && output.status.success()
    {
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Ok(child_pid) = line.trim().parse::<u32>() {
                collect_descendants(child_pid, pids);
                pids.push(child_pid);
            }
        }
    }
}

/// Kills a child process and all of its descendants.
///
/// On Unix, first recursively collects all descendant PIDs (via `pgrep -P`)
/// while parent-child links are intact, then kills them deepest-first, and
/// finally kills the process group as a belt-and-suspenders measure. This
/// catches descendants that escaped the process group via `setsid()`.
///
/// On Windows, uses `taskkill /T /F` to kill the process tree.
///
/// Blocks until the direct child has exited.
pub fn kill_process_tree(child: &mut Child) {
    let pid = child.id();

    #[cfg(unix)]
    {
        // First, recursively collect all descendant PIDs while the
        // parent-child links are still intact. This catches processes
        // that called setsid() and left the process group (e.g.
        // sshd-session with PTY allocation, daemonized servers).
        let mut descendants = Vec::new();
        collect_descendants(pid, &mut descendants);

        // Kill descendants deepest-first (collected in post-order).
        for &desc_pid in &descendants {
            // SAFETY: Simple signal-sending syscall with a valid PID.
            unsafe {
                libc::kill(desc_pid as i32, libc::SIGKILL);
            }
        }

        // Kill the process group to catch any remaining members.
        // The child was spawned with process_group(0), making its PID
        // the process group ID. Negating the PID targets the whole group.
        //
        // SAFETY: Simple signal-sending syscall with a valid (negated) PID.
        unsafe {
            libc::kill(-(pid as i32), libc::SIGKILL);
        }
    }

    #[cfg(windows)]
    {
        use std::process::{Command, Stdio};
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    let _ = child.wait();
}
