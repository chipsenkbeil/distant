//! A minimal TCP echo server for tunnel integration tests.
//!
//! Listens on `127.0.0.1:0` (OS-assigned port), prints the assigned port
//! number to stdout as its first line, then accepts a single connection and
//! echoes all received data back until EOF or timeout.
//!
//! Usage: `tcp-echo-server [TIMEOUT_SECS]` (default: 30)

#[tokio::main]
async fn main() {
    let timeout_secs: u64 = std::env::args()
        .nth(1)
        .map(|s| s.parse().expect("TIMEOUT_SECS must be a valid u64"))
        .unwrap_or(30);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("failed to bind to 127.0.0.1:0");

    let port = listener
        .local_addr()
        .expect("failed to get local address")
        .port();
    println!("{port}");

    let accept = async {
        let (stream, _addr) = listener
            .accept()
            .await
            .expect("failed to accept connection");
        stream
    };

    let stream = tokio::select! {
        s = accept => s,
        () = tokio::time::sleep(std::time::Duration::from_secs(timeout_secs)) => {
            eprintln!("timeout waiting for connection after {timeout_secs}s");
            return;
        }
    };

    let (mut reader, mut writer) = tokio::io::split(stream);

    tokio::select! {
        r = tokio::io::copy(&mut reader, &mut writer) => {
            if let Err(e) = r {
                eprintln!("echo error: {e}");
            }
        }
        () = tokio::time::sleep(std::time::Duration::from_secs(timeout_secs)) => {
            eprintln!("timeout during echo after {timeout_secs}s");
        }
    }
}
