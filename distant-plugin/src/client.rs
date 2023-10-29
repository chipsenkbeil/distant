use std::io;
use std::sync::mpsc;

use async_trait::async_trait;
use distant_core_protocol::{Request, Response};

/// Full API for a distant-compatible client.
#[async_trait]
pub trait Client {
    /// Sends a request without waiting for a response; this method is able to be used even
    /// if the session's receiving line to the remote server has been severed.
    async fn fire(&mut self, request: Request) -> io::Result<()>;

    /// Sends a request and returns a mailbox that can receive one or more responses, failing if
    /// unable to send a request or if the session's receiving line to the remote server has
    /// already been severed.
    async fn mail(&mut self, request: Request) -> io::Result<mpsc::Receiver<Response>>;

    /// Sends a request and waits for a response, failing if unable to send a request or if
    /// the session's receiving line to the remote server has already been severed
    async fn send(&mut self, request: Request) -> io::Result<Response>;
}
