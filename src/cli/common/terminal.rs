//! Terminal utilities for size detection, raw mode, and resize events.

/// Returns the current terminal size as `(cols, rows)`, or `None` if unavailable.
#[cfg(unix)]
pub fn terminal_size() -> Option<(u16, u16)> {
    use libc::{STDOUT_FILENO, TIOCGWINSZ, ioctl, winsize};

    unsafe {
        let mut ws: winsize = std::mem::zeroed();
        if ioctl(STDOUT_FILENO, TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 && ws.ws_row > 0 {
            Some((ws.ws_col, ws.ws_row))
        } else {
            None
        }
    }
}

/// Returns the current terminal size as `(cols, rows)`, or `None` if unavailable.
#[cfg(windows)]
pub fn terminal_size() -> Option<(u16, u16)> {
    use windows_sys::Win32::System::Console::{
        CONSOLE_SCREEN_BUFFER_INFO, GetConsoleScreenBufferInfo, GetStdHandle, STD_OUTPUT_HANDLE,
    };

    unsafe {
        let handle = GetStdHandle(STD_OUTPUT_HANDLE);
        if handle.is_null() {
            return None;
        }
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = std::mem::zeroed();
        if GetConsoleScreenBufferInfo(handle, &mut info) != 0 {
            let cols = (info.srWindow.Right - info.srWindow.Left + 1) as u16;
            let rows = (info.srWindow.Bottom - info.srWindow.Top + 1) as u16;
            if cols > 0 && rows > 0 {
                Some((cols, rows))
            } else {
                None
            }
        } else {
            None
        }
    }
}

/// RAII guard that sets the terminal to raw mode and restores it on drop.
#[cfg(unix)]
pub struct RawMode {
    original: libc::termios,
}

#[cfg(unix)]
impl RawMode {
    /// Enters raw mode on stdin. Returns a guard that restores the original state on drop.
    pub fn enter() -> std::io::Result<Self> {
        unsafe {
            let mut original: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut original) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            let mut raw = original;
            libc::cfmakeraw(&mut raw);
            if libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(Self { original })
        }
    }
}

#[cfg(unix)]
impl Drop for RawMode {
    fn drop(&mut self) {
        unsafe {
            let _ = libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.original);
        }
    }
}

/// RAII guard that sets the terminal to raw mode and restores it on drop.
#[cfg(windows)]
pub struct RawMode {
    original_input: u32,
    original_output: u32,
}

#[cfg(windows)]
impl RawMode {
    /// Enters raw mode on the console. Returns a guard that restores the original state on drop.
    pub fn enter() -> std::io::Result<Self> {
        use windows_sys::Win32::System::Console::{
            ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_INPUT,
            ENABLE_VIRTUAL_TERMINAL_PROCESSING, ENABLE_WRAP_AT_EOL_OUTPUT, GetConsoleMode,
            GetStdHandle, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetConsoleMode,
        };

        unsafe {
            let h_in = GetStdHandle(STD_INPUT_HANDLE);
            let h_out = GetStdHandle(STD_OUTPUT_HANDLE);

            let mut original_input: u32 = 0;
            let mut original_output: u32 = 0;
            if GetConsoleMode(h_in, &mut original_input) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            if GetConsoleMode(h_out, &mut original_output) == 0 {
                return Err(std::io::Error::last_os_error());
            }

            // Enable virtual terminal input (raw escape sequences from stdin)
            let raw_input = ENABLE_VIRTUAL_TERMINAL_INPUT;
            if SetConsoleMode(h_in, raw_input) == 0 {
                return Err(std::io::Error::last_os_error());
            }

            // Enable VT processing on output
            let raw_output = ENABLE_PROCESSED_OUTPUT
                | ENABLE_WRAP_AT_EOL_OUTPUT
                | ENABLE_VIRTUAL_TERMINAL_PROCESSING;
            if SetConsoleMode(h_out, raw_output) == 0 {
                // Restore input mode before returning error
                let _ = SetConsoleMode(h_in, original_input);
                return Err(std::io::Error::last_os_error());
            }

            Ok(Self {
                original_input,
                original_output,
            })
        }
    }
}

#[cfg(windows)]
impl Drop for RawMode {
    fn drop(&mut self) {
        use windows_sys::Win32::System::Console::{
            GetStdHandle, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetConsoleMode,
        };

        unsafe {
            let h_in = GetStdHandle(STD_INPUT_HANDLE);
            let h_out = GetStdHandle(STD_OUTPUT_HANDLE);
            let _ = SetConsoleMode(h_in, self.original_input);
            let _ = SetConsoleMode(h_out, self.original_output);
        }
    }
}

/// Waits for the next terminal resize event, returning the new `(cols, rows)`.
///
/// On Unix, this awaits SIGWINCH. On Windows, this polls every 500ms.
#[cfg(unix)]
pub async fn wait_for_resize() -> Option<(u16, u16)> {
    use tokio::signal::unix::{SignalKind, signal};

    let mut sigwinch = signal(SignalKind::window_change()).ok()?;
    sigwinch.recv().await?;
    terminal_size()
}

/// Waits for the next terminal resize event, returning the new `(cols, rows)`.
///
/// On Windows, this polls every 500ms for size changes.
#[cfg(windows)]
pub async fn wait_for_resize() -> Option<(u16, u16)> {
    use std::time::Duration;

    let current = terminal_size();
    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let new = terminal_size();
        if new != current {
            return new;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_size_returns_some_or_none() {
        // In a real terminal this returns Some, in CI/piped it returns None.
        // Either way it must not panic.
        match terminal_size() {
            Some((cols, rows)) => {
                assert!(cols > 0);
                assert!(rows > 0);
            }
            None => {
                // Expected when not attached to a terminal (e.g. CI)
            }
        }
    }

    /// Verifies that RawMode restores terminal state on drop.
    ///
    /// This test only runs on Unix when stdin is a TTY (skipped in CI/piped).
    #[cfg(unix)]
    #[test]
    fn raw_mode_restores_terminal_on_drop() {
        unsafe {
            // Only run if stdin is a TTY
            if libc::isatty(libc::STDIN_FILENO) == 0 {
                return;
            }

            // Save original state
            let mut before: libc::termios = std::mem::zeroed();
            libc::tcgetattr(libc::STDIN_FILENO, &mut before);

            // Enter and immediately drop raw mode
            {
                let _guard = RawMode::enter().expect("Failed to enter raw mode");
                // While in raw mode, termios should differ from original
            }

            // After drop, state should be restored
            let mut after: libc::termios = std::mem::zeroed();
            libc::tcgetattr(libc::STDIN_FILENO, &mut after);

            assert_eq!(before.c_lflag, after.c_lflag);
            assert_eq!(before.c_iflag, after.c_iflag);
            assert_eq!(before.c_oflag, after.c_oflag);
            assert_eq!(before.c_cflag, after.c_cflag);
        }
    }
}
