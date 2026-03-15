//! Password prompt scenario for PTY predictive echo testing.
//!
//! Prints a password prompt, reads a password with echo disabled
//! (via `rpassword`), then resumes a byte-by-byte echo loop.
//! Used to verify that the prediction engine detects no-echo mode.

use std::io::{Read, Write};

fn main() {
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    let _ = write!(stdout, "Password: ");
    let _ = stdout.flush();

    // Read password with echo disabled.
    let _password = rpassword::read_password().unwrap_or_default();

    let _ = writeln!(stdout, "\nAuthenticated.");
    let _ = stdout.flush();

    // Resume echo loop.
    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();
    let mut buf = [0u8; 1];
    while stdin.read(&mut buf).unwrap_or(0) == 1 {
        let _ = stdout.write_all(&buf);
        let _ = stdout.flush();
    }
}
