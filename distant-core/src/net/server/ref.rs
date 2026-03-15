use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::sync::broadcast;
use tokio::task::{JoinError, JoinHandle};

/// Represents a reference to a server.
pub struct ServerRef {
    pub(crate) shutdown: broadcast::Sender<()>,
    pub(crate) task: JoinHandle<()>,
}

impl ServerRef {
    /// Returns `true` if the server task has completed.
    pub fn is_finished(&self) -> bool {
        self.task.is_finished()
    }

    /// Sends a shutdown signal to the server, causing it to terminate gracefully.
    pub fn shutdown(&self) {
        let _ = self.shutdown.send(());
    }

    /// Returns a receiver that is notified when [`shutdown`](Self::shutdown) is called.
    pub fn subscribe_shutdown(&self) -> broadcast::Receiver<()> {
        self.shutdown.subscribe()
    }

    /// Returns a lightweight handle that can trigger server shutdown.
    ///
    /// Unlike [`ServerRef`] itself, [`ShutdownSender`] is [`Clone`] and [`Send`],
    /// making it suitable for passing to background health-monitoring tasks that
    /// need to shut down the server when a backend dies.
    pub fn shutdown_sender(&self) -> ShutdownSender {
        ShutdownSender {
            sender: self.shutdown.clone(),
        }
    }
}

/// A lightweight, cloneable handle for triggering server shutdown.
///
/// Obtained via [`ServerRef::shutdown_sender`]. Calling [`shutdown`](Self::shutdown)
/// sends the same signal as [`ServerRef::shutdown`].
#[derive(Clone)]
pub struct ShutdownSender {
    sender: broadcast::Sender<()>,
}

impl ShutdownSender {
    /// Sends the shutdown signal to the associated server.
    pub fn shutdown(&self) {
        let _ = self.sender.send(());
    }
}

impl Future for ServerRef {
    type Output = Result<(), JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.task).poll(cx)
    }
}

mod tcp;
pub use tcp::*;

#[cfg(unix)]
mod unix;

#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn make_server_ref() -> ServerRef {
        let (shutdown, _) = broadcast::channel(1);
        let task = tokio::spawn(async {});
        ServerRef { shutdown, task }
    }

    // ---------------------------------------------------------------
    // ShutdownSender
    // ---------------------------------------------------------------

    #[test_log::test(tokio::test)]
    async fn shutdown_sender_triggers_receiver() {
        let (tx, _) = broadcast::channel::<()>(1);
        let mut rx = tx.subscribe();
        let sender = ShutdownSender { sender: tx };

        sender.shutdown();

        let result = rx.recv().await;
        assert_eq!(result.unwrap(), ());
    }

    #[test_log::test(tokio::test)]
    async fn shutdown_sender_clone_triggers_same_channel() {
        let (tx, _) = broadcast::channel::<()>(1);
        let mut rx = tx.subscribe();
        let sender = ShutdownSender { sender: tx };
        let cloned = sender.clone();

        // Use the clone to send
        cloned.shutdown();

        let result = rx.recv().await;
        assert_eq!(result.unwrap(), ());
    }

    #[test_log::test(tokio::test)]
    async fn shutdown_sender_original_and_clone_both_work() {
        let (tx, _) = broadcast::channel::<()>(2);
        let mut rx = tx.subscribe();
        let sender = ShutdownSender { sender: tx };
        let cloned = sender.clone();

        sender.shutdown();
        let first = rx.recv().await;
        assert_eq!(first.unwrap(), ());

        cloned.shutdown();
        let second = rx.recv().await;
        assert_eq!(second.unwrap(), ());
    }

    #[test_log::test(tokio::test)]
    async fn shutdown_sender_does_not_panic_when_no_receivers() {
        let (tx, _) = broadcast::channel::<()>(1);
        let sender = ShutdownSender { sender: tx };
        // No receiver subscribed — shutdown should not panic
        sender.shutdown();
    }

    // ---------------------------------------------------------------
    // ServerRef::shutdown_sender
    // ---------------------------------------------------------------

    #[test_log::test(tokio::test)]
    async fn server_ref_shutdown_sender_returns_working_sender() {
        let server_ref = make_server_ref();
        let mut rx = server_ref.subscribe_shutdown();
        let sender = server_ref.shutdown_sender();

        sender.shutdown();

        let result = rx.recv().await;
        assert_eq!(result.unwrap(), ());
    }

    #[test_log::test(tokio::test)]
    async fn server_ref_shutdown_sender_shares_channel_with_server_ref() {
        let server_ref = make_server_ref();
        let sender = server_ref.shutdown_sender();
        let mut rx = server_ref.subscribe_shutdown();

        // Shutting down via the sender should be equivalent to server_ref.shutdown()
        sender.shutdown();

        let result = rx.recv().await;
        assert_eq!(result.unwrap(), ());
    }

    #[test_log::test(tokio::test)]
    async fn server_ref_shutdown_via_sender_stops_server_task() {
        // Create a server ref with a long-running task that listens for shutdown
        let (shutdown_tx, _) = broadcast::channel(1);
        let mut shutdown_rx = shutdown_tx.subscribe();
        let task = tokio::spawn(async move {
            let _ = shutdown_rx.recv().await;
        });
        let server_ref = ServerRef {
            shutdown: shutdown_tx,
            task,
        };

        assert!(!server_ref.is_finished());

        let sender = server_ref.shutdown_sender();
        sender.shutdown();

        // Wait for the task to notice and finish
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(server_ref.is_finished());
    }
}
