//! Mini-shell for PTY-based predictive echo testing.
//!
//! Provides controlled scenarios that simulate real shell behavior
//! (echo, password prompts, signal handling) so the prediction engine
//! can be tested against actual PTY byte streams.

#[cfg(unix)]
fn main() {
    let scenario = std::env::args().nth(1).unwrap_or_else(|| "echo".into());
    match scenario.as_str() {
        "echo" => scenario_echo(),
        "password" => scenario_password(),
        "interactive" => scenario_interactive(),
        other => {
            eprintln!("unknown scenario: {other}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!("predict-pty-helper is unix-only");
    std::process::exit(1);
}

/// Simple read-echo loop. Reads stdin byte-by-byte and writes each
/// byte back to stdout. Baseline for prediction confirmation tests.
#[cfg(unix)]
fn scenario_echo() {
    use std::io::{Read, Write};
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut stdin = stdin.lock();
    let mut stdout = stdout.lock();
    let mut buf = [0u8; 1];
    while stdin.read(&mut buf).unwrap_or(0) == 1 {
        // Echo back what we received.
        let _ = stdout.write_all(&buf);
        let _ = stdout.flush();
    }
}

/// Password scenario: prompt, read password (no echo), then resume echo.
#[cfg(unix)]
fn scenario_password() {
    use std::io::{Read, Write};

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

#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(unix)]
static GOT_SIGINT: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn sigint_handler(_sig: libc::c_int) {
    GOT_SIGINT.store(true, Ordering::Relaxed);
}

/// Interactive mini-shell with password command and signal handling.
#[cfg(unix)]
fn scenario_interactive() {
    use std::io::{BufRead, Write};

    // Install SIGINT handler that sets a flag instead of killing.
    unsafe {
        libc::signal(
            libc::SIGINT,
            sigint_handler as *const () as libc::sighandler_t,
        );
    }

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();

    let mut stdout_lock = stdout.lock();
    let _ = write!(stdout_lock, "$ ");
    let _ = stdout_lock.flush();
    drop(stdout_lock);

    let stdin_lock = stdin.lock();
    for line in stdin_lock.lines() {
        // Check if SIGINT was received during line reading.
        if GOT_SIGINT.swap(false, Ordering::Relaxed) {
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
