//! Bridges stdin/stdout to a TCP socket.
//!
//! Used as a portable `ProxyCommand` replacement in integration tests.
//! Takes `host:port` as its single argument, connects via TCP, then
//! relays data bidirectionally between stdin/stdout and the socket.

#[tokio::main]
async fn main() {
    let addr = std::env::args()
        .nth(1)
        .expect("usage: tcp-to-stdio host:port");

    let tcp = tokio::net::TcpStream::connect(&addr)
        .await
        .unwrap_or_else(|e| panic!("failed to connect to {addr}: {e}"));

    let (mut tcp_r, mut tcp_w) = tokio::io::split(tcp);
    let mut stdin = tokio::io::stdin();
    let mut stdout = tokio::io::stdout();

    tokio::select! {
        r = tokio::io::copy(&mut stdin, &mut tcp_w) => {
            if let Err(e) = r {
                eprintln!("stdin→tcp error: {e}");
            }
        }
        r = tokio::io::copy(&mut tcp_r, &mut stdout) => {
            if let Err(e) = r {
                eprintln!("tcp→stdout error: {e}");
            }
        }
    }
}
