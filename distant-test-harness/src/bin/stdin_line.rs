//! Reads one line from stdin, writes it to stdout, and exits.
//! Cross-platform replacement for `head -1` / `findstr` in stdin forwarding tests.

use std::io::{BufRead, Write};

fn main() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut line = String::new();
    if stdin.lock().read_line(&mut line).unwrap_or(0) > 0 {
        let _ = stdout.lock().write_all(line.as_bytes());
        let _ = stdout.lock().flush();
    }
}
