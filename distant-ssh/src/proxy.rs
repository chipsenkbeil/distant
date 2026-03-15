//! ProxyCommand support for SSH connections.
//!
//! Provides `ProxyStream`, a wrapper around a child process whose stdin/stdout
//! are used as the transport for an SSH connection, mirroring OpenSSH's
//! `ProxyCommand` behavior.

use std::io;
use std::pin::Pin;
use std::process::Stdio;
use std::task::{Context, Poll};

use log::*;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

/// Perform `%h`, `%p`, `%r`, and `%%` substitution on a ProxyCommand template.
///
/// Tokens:
/// - `%h` — target hostname
/// - `%p` — target port
/// - `%r` — remote username
/// - `%%` — literal `%`
///
/// Unknown `%x` sequences are passed through unchanged.
pub fn substitute_proxy_command(template: &str, host: &str, port: u16, user: &str) -> String {
    let mut result = String::with_capacity(template.len());
    let mut chars = template.chars().peekable();

    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.peek() {
                Some('h') => {
                    chars.next();
                    result.push_str(host);
                }
                Some('p') => {
                    chars.next();
                    result.push_str(&port.to_string());
                }
                Some('r') => {
                    chars.next();
                    result.push_str(user);
                }
                Some('%') => {
                    chars.next();
                    result.push('%');
                }
                _ => {
                    // Unknown token — pass through unchanged
                    result.push('%');
                }
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// A bidirectional stream backed by a child process's stdin/stdout.
///
/// Used as the transport for `russh::client::connect_stream` when a
/// ProxyCommand is configured. The child process is killed on drop.
pub struct ProxyStream {
    child: Child,
    stdin: ChildStdin,
    stdout: ChildStdout,
}

impl ProxyStream {
    /// Spawn a ProxyCommand and return a stream connected to its stdin/stdout.
    ///
    /// On Unix, runs `sh -c "$cmd"`. On Windows, runs `cmd /C "$cmd"`.
    ///
    /// # Errors
    ///
    /// Returns an error if the child process fails to spawn, or if its
    /// stdin/stdout pipes cannot be captured.
    pub fn spawn(cmd: &str) -> io::Result<Self> {
        debug!("Spawning ProxyCommand: {}", cmd);

        #[cfg(unix)]
        let mut command = {
            let mut c = Command::new("sh");
            c.arg("-c").arg(cmd);
            c
        };

        #[cfg(windows)]
        let mut command = {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(cmd);
            c
        };

        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());

        let mut child = command.spawn().map_err(|e| {
            io::Error::new(
                e.kind(),
                format!("Failed to spawn ProxyCommand '{cmd}': {e}"),
            )
        })?;

        let stdin = child.stdin.take().ok_or_else(|| {
            io::Error::new(io::ErrorKind::BrokenPipe, "ProxyCommand stdin not captured")
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "ProxyCommand stdout not captured",
            )
        })?;

        Ok(Self {
            child,
            stdin,
            stdout,
        })
    }
}

impl AsyncRead for ProxyStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stdout).poll_read(cx, buf)
    }
}

impl AsyncWrite for ProxyStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.stdin).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stdin).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.stdin).poll_shutdown(cx)
    }
}

impl Drop for ProxyStream {
    fn drop(&mut self) {
        if let Err(e) = self.child.start_kill() {
            debug!("Failed to kill ProxyCommand process: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitute_proxy_command_should_replace_host() {
        assert_eq!(
            substitute_proxy_command("ssh %h", "example.com", 22, "user"),
            "ssh example.com"
        );
    }

    #[test]
    fn substitute_proxy_command_should_replace_port() {
        assert_eq!(
            substitute_proxy_command("nc %h %p", "host", 2222, "user"),
            "nc host 2222"
        );
    }

    #[test]
    fn substitute_proxy_command_should_replace_user() {
        assert_eq!(
            substitute_proxy_command("ssh -l %r %h", "host", 22, "admin"),
            "ssh -l admin host"
        );
    }

    #[test]
    fn substitute_proxy_command_should_replace_percent_literal() {
        assert_eq!(
            substitute_proxy_command("echo %%", "host", 22, "user"),
            "echo %"
        );
    }

    #[test]
    fn substitute_proxy_command_should_handle_mixed_tokens() {
        assert_eq!(
            substitute_proxy_command(
                "exec x2ssh -fallback -tunnel %h -p %p -u %r %%done",
                "devvm.example.com",
                22,
                "root"
            ),
            "exec x2ssh -fallback -tunnel devvm.example.com -p 22 -u root %done"
        );
    }

    #[test]
    fn substitute_proxy_command_should_pass_unknown_tokens_through() {
        assert_eq!(
            substitute_proxy_command("test %x %h", "host", 22, "user"),
            "test %x host"
        );
    }

    #[test]
    fn substitute_proxy_command_should_handle_trailing_percent() {
        assert_eq!(
            substitute_proxy_command("test %", "host", 22, "user"),
            "test %"
        );
    }

    #[test]
    fn substitute_proxy_command_should_handle_empty_template() {
        assert_eq!(substitute_proxy_command("", "host", 22, "user"), "");
    }

    #[test]
    fn substitute_proxy_command_should_return_unchanged_without_tokens() {
        assert_eq!(
            substitute_proxy_command("nc proxy.internal 443", "host", 22, "user"),
            "nc proxy.internal 443"
        );
    }

    #[tokio::test]
    async fn proxy_stream_should_read_stdout() {
        use tokio::io::AsyncReadExt;

        let mut stream = ProxyStream::spawn("echo hello").unwrap();
        let mut buf = vec![0u8; 64];
        let n = stream.read(&mut buf).await.unwrap();
        let output = String::from_utf8_lossy(&buf[..n]);
        assert!(
            output.contains("hello"),
            "Expected 'hello', got: {output:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn proxy_stream_should_echo_stdin_to_stdout() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        // `head -c 4` reads exactly 4 bytes then exits, avoiding the need
        // to close stdin (AsyncWrite::shutdown on ChildStdin is a no-op).
        let mut stream = ProxyStream::spawn("head -c 4").unwrap();
        stream.write_all(b"ping").await.unwrap();

        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        assert_eq!(buf, b"ping");
    }
}
