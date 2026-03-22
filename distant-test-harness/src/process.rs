//! Process management utilities for test cleanup.
//!
//! Provides cross-platform helpers to spawn processes in their own process
//! group and kill entire process trees, preventing leaked handles detected
//! by nextest.

use std::io;
use std::mem::ManuallyDrop;
use std::ops;
use std::process::{Child, Command, Output, Stdio};

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

/// RAII wrapper around a child process that ensures cleanup on drop.
///
/// When a `TestChild` is dropped — including during a test panic unwind — it
/// kills the wrapped process and all of its descendants via [`kill_process_tree`],
/// then waits for the direct child to exit. This prevents leaked processes from
/// causing nextest handle-leak failures or accumulating between test runs.
///
/// Use [`Deref`]/[`DerefMut`] to access the underlying [`Child`] fields
/// (`.stdin`, `.stdout`, `.stderr`) and methods (`.try_wait()`, etc.).
pub struct TestChild {
    inner: ManuallyDrop<Child>,
}

impl TestChild {
    /// Spawns a command as a new child process with piped stdio.
    ///
    /// The command is configured to run in its own process group (on Unix) so
    /// that [`kill_process_tree`] can reliably kill all descendants. All three
    /// stdio handles are piped for test inspection.
    pub fn spawn(cmd: &mut Command) -> io::Result<Self> {
        set_process_group(cmd);
        let child = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn()?;

        Ok(Self {
            inner: ManuallyDrop::new(child),
        })
    }

    /// Consumes the wrapper and waits for the child to finish, collecting its
    /// output.
    ///
    /// This bypasses the kill-on-drop behavior, allowing the child to exit
    /// naturally.
    pub fn wait_with_output(mut self) -> io::Result<Output> {
        // SAFETY: We take ownership of the inner Child before forgetting self,
        // so Drop will not run and no double-free occurs.
        let child = unsafe { ManuallyDrop::take(&mut self.inner) };
        std::mem::forget(self);
        child.wait_with_output()
    }

    /// Explicitly kills the child process tree and consumes the wrapper.
    ///
    /// Equivalent to the automatic cleanup on drop, but makes the intent
    /// explicit.
    pub fn kill(self) {
        drop(self);
    }
}

impl ops::Deref for TestChild {
    type Target = Child;

    fn deref(&self) -> &Child {
        &self.inner
    }
}

impl ops::DerefMut for TestChild {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.inner
    }
}

impl Drop for TestChild {
    fn drop(&mut self) {
        // SAFETY: This runs exactly once during drop. After taking, the
        // ManuallyDrop is consumed and no further access occurs.
        let mut child = unsafe { ManuallyDrop::take(&mut self.inner) };
        kill_process_tree(&mut child);
    }
}
