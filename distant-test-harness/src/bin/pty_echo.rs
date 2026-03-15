//! Simple byte-by-byte echo loop for PTY predictive echo testing.
//!
//! Reads stdin one byte at a time and writes each byte back to stdout.
//! Used as a baseline for prediction confirmation tests.

use std::io::{Read, Write};

fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut stdin = stdin.lock();
    let mut stdout = stdout.lock();
    let mut buf = [0u8; 1];
    while stdin.read(&mut buf).unwrap_or(0) == 1 {
        let _ = stdout.write_all(&buf);
        let _ = stdout.flush();
    }
}
