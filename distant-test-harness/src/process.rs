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

/// Kills a child process and all of its descendants.
///
/// On Unix, sends `SIGKILL` to the entire process group (requires the child to
/// have been spawned with [`set_process_group`]).
///
/// On Windows, uses `taskkill /T /F` to kill the process tree.
///
/// Blocks until the direct child has exited.
pub fn kill_process_tree(child: &mut Child) {
    let pid = child.id();

    #[cfg(unix)]
    {
        // Kill the entire process group. The child was spawned with
        // process_group(0), making its PID the process group ID.
        // Negating the PID targets the whole group.
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
