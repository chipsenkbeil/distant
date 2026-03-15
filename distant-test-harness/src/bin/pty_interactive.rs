//! Interactive mini-shell for PTY predictive echo testing.
//!
//! Provides a `$ ` prompt, recognizes `exit` and `passwd` commands,
//! and handles Ctrl+C via the `ctrlc` crate. Used to test prediction
//! behavior across prompt boundaries, password input, and signal delivery.

use std::io::{BufRead, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

fn main() {
    let got_sigint = Arc::new(AtomicBool::new(false));

    // Install cross-platform Ctrl+C handler.
    {
        let flag = Arc::clone(&got_sigint);
        ctrlc::set_handler(move || {
            flag.store(true, Ordering::Relaxed);
        })
        .expect("failed to set Ctrl+C handler");
    }

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();

    let mut stdout_lock = stdout.lock();
    let _ = write!(stdout_lock, "$ ");
    let _ = stdout_lock.flush();
    drop(stdout_lock);

    let stdin_lock = stdin.lock();
    for line in stdin_lock.lines() {
        // Check if Ctrl+C was received during line reading.
        if got_sigint.swap(false, Ordering::Relaxed) {
            let mut stdout_lock = stdout.lock();
            let _ = write!(stdout_lock, "\n$ ");
            let _ = stdout_lock.flush();
            continue;
        }

        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        let cmd = line.trim();
        if cmd == "exit" {
            break;
        }

        if cmd == "passwd" {
            let mut stdout_lock = stdout.lock();
            let _ = write!(stdout_lock, "Password: ");
            let _ = stdout_lock.flush();
            drop(stdout_lock);

            // Read password with echo disabled.
            let _password = rpassword::read_password().unwrap_or_default();

            let mut stdout_lock = stdout.lock();
            let _ = writeln!(stdout_lock, "OK");
            let _ = write!(stdout_lock, "$ ");
            let _ = stdout_lock.flush();
            continue;
        }

        let mut stdout_lock = stdout.lock();
        let _ = writeln!(stdout_lock, "{cmd}: done");
        let _ = write!(stdout_lock, "$ ");
        let _ = stdout_lock.flush();
    }
}
