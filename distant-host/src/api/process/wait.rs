use tokio::io;
use tokio::sync::mpsc;

/// Exit status of a remote process
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ExitStatus {
    pub success: bool,
    pub code: Option<i32>,
}

impl ExitStatus {
    /// Produces a new exit status representing a killed process
    pub fn killed() -> Self {
        Self {
            success: false,
            code: None,
        }
    }
}

impl<T, E> From<Result<T, E>> for ExitStatus
where
    T: Into<ExitStatus>,
    E: Into<ExitStatus>,
{
    fn from(res: Result<T, E>) -> Self {
        match res {
            Ok(x) => x.into(),
            Err(x) => x.into(),
        }
    }
}

impl From<io::Error> for ExitStatus {
    fn from(err: io::Error) -> Self {
        Self {
            success: false,
            code: err.raw_os_error(),
        }
    }
}

impl From<std::process::ExitStatus> for ExitStatus {
    fn from(status: std::process::ExitStatus) -> Self {
        Self {
            success: status.success(),
            code: status.code(),
        }
    }
}

/// Creates a new channel for when the exit status will be ready
pub fn channel() -> (WaitTx, WaitRx) {
    let (tx, rx) = mpsc::channel(1);
    (WaitTx::Pending(tx), WaitRx::Pending(rx))
}

/// Represents a notifier for a specific waiting state
#[derive(Debug)]
pub enum WaitTx {
    /// Notification has been sent
    Done,

    /// Notification has not been sent
    Pending(mpsc::Sender<ExitStatus>),
}

impl WaitTx {
    /// Send exit status to receiving-side of wait
    pub async fn send<S>(&mut self, status: S) -> io::Result<()>
    where
        S: Into<ExitStatus>,
    {
        let status = status.into();

        match self {
            Self::Done => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Notifier is closed",
            )),
            Self::Pending(tx) => {
                let res = tx.send(status).await;
                *self = Self::Done;

                match res {
                    Ok(_) => Ok(()),
                    Err(x) => Err(io::Error::other(x)),
                }
            }
        }
    }

    /// Mark wait as completed using killed status
    pub async fn kill(&mut self) -> io::Result<()> {
        self.send(ExitStatus::killed()).await
    }
}

/// Represents the state of waiting for an exit status
#[derive(Debug)]
pub enum WaitRx {
    /// Exit status is ready
    Ready(ExitStatus),

    /// If receiver for an exit status has been dropped without receiving the status
    Dropped,

    /// Exit status is not ready and has a "oneshot" to be invoked when available
    Pending(mpsc::Receiver<ExitStatus>),
}

impl WaitRx {
    /// Waits until the exit status is resolved; can be called repeatedly after being
    /// resolved to immediately return the exit status again
    pub async fn recv(&mut self) -> io::Result<ExitStatus> {
        match self {
            Self::Ready(status) => Ok(*status),
            Self::Dropped => Err(io::Error::other("Internal resolver dropped")),
            Self::Pending(rx) => match rx.recv().await {
                Some(status) => {
                    *self = Self::Ready(status);
                    Ok(status)
                }
                None => {
                    *self = Self::Dropped;
                    Err(io::Error::other("Internal resolver dropped"))
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    //! Tests for `ExitStatus` (constructors, From impls, equality/copy) and the
    //! `WaitTx`/`WaitRx` channel pair covering all state transitions.

    use super::*;

    mod exit_status {
        use super::*;

        #[test]
        fn killed_should_have_success_false_and_code_none() {
            let status = ExitStatus::killed();
            assert!(!status.success);
            assert_eq!(status.code, None);
        }

        #[test]
        fn from_io_error_with_raw_os_error_should_set_code() {
            let err = io::Error::from_raw_os_error(42);
            let status = ExitStatus::from(err);
            assert!(!status.success);
            assert_eq!(status.code, Some(42));
        }

        #[test]
        fn from_io_error_without_raw_os_error_should_have_code_none() {
            let err = io::Error::other("custom error");
            let status = ExitStatus::from(err);
            assert!(!status.success);
            assert_eq!(status.code, None);
        }

        #[test]
        fn from_result_ok_should_use_ok_value() {
            let success_status = ExitStatus {
                success: true,
                code: Some(0),
            };
            let res: Result<ExitStatus, ExitStatus> = Ok(success_status);
            let status = ExitStatus::from(res);
            assert!(status.success);
            assert_eq!(status.code, Some(0));
        }

        #[test]
        fn from_result_err_should_use_err_value() {
            let fail_status = ExitStatus {
                success: false,
                code: Some(1),
            };
            let res: Result<ExitStatus, ExitStatus> = Err(fail_status);
            let status = ExitStatus::from(res);
            assert!(!status.success);
            assert_eq!(status.code, Some(1));
        }

        #[test]
        fn from_result_ok_with_io_error_inner_uses_err_conversion() {
            // Ok(io::Error) should convert the io::Error into ExitStatus
            let err = io::Error::from_raw_os_error(99);
            let res: Result<io::Error, ExitStatus> = Ok(err);
            let status = ExitStatus::from(res);
            assert!(!status.success);
            assert_eq!(status.code, Some(99));
        }

        #[test]
        fn from_result_err_with_io_error_uses_io_error_conversion() {
            let err = io::Error::from_raw_os_error(13);
            let res: Result<ExitStatus, io::Error> = Err(err);
            let status = ExitStatus::from(res);
            assert!(!status.success);
            assert_eq!(status.code, Some(13));
        }

        #[test]
        fn equality_and_copy() {
            // Renamed from equality_and_clone: the test uses `let b = a` which is
            // a Copy (not Clone::clone), matching ExitStatus's #[derive(Copy)] semantics.
            let a = ExitStatus {
                success: true,
                code: Some(0),
            };
            let b = a; // Copy, not Clone
            assert_eq!(a, b);

            let c = ExitStatus {
                success: false,
                code: Some(1),
            };
            assert_ne!(a, c);
        }
    }

    mod wait_channel {
        use super::*;

        #[test]
        fn channel_creates_pending_pair() {
            let (tx, rx) = channel();
            assert!(matches!(tx, WaitTx::Pending(_)));
            assert!(matches!(rx, WaitRx::Pending(_)));
        }

        #[test_log::test(tokio::test)]
        async fn send_transitions_tx_to_done() {
            let (mut tx, _rx) = channel();
            let status = ExitStatus {
                success: true,
                code: Some(0),
            };
            tx.send(status).await.unwrap();
            assert!(matches!(tx, WaitTx::Done));
        }

        #[test_log::test(tokio::test)]
        async fn send_on_done_returns_broken_pipe_error() {
            let (mut tx, _rx) = channel();
            let status = ExitStatus {
                success: true,
                code: Some(0),
            };
            tx.send(status).await.unwrap();

            // Second send should fail because tx is now Done
            let result = tx.send(status).await;
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
        }

        #[test_log::test(tokio::test)]
        async fn kill_sends_killed_status() {
            let (mut tx, mut rx) = channel();
            tx.kill().await.unwrap();

            let status = rx.recv().await.unwrap();
            assert_eq!(status, ExitStatus::killed());
            assert!(!status.success);
            assert_eq!(status.code, None);
        }

        #[test_log::test(tokio::test)]
        async fn recv_returns_sent_status() {
            let (mut tx, mut rx) = channel();
            let expected = ExitStatus {
                success: true,
                code: Some(42),
            };
            tx.send(expected).await.unwrap();

            let status = rx.recv().await.unwrap();
            assert_eq!(status, expected);
        }

        #[test_log::test(tokio::test)]
        async fn recv_transitions_to_ready_and_subsequent_calls_return_same_status() {
            let (mut tx, mut rx) = channel();
            let expected = ExitStatus {
                success: false,
                code: Some(7),
            };
            tx.send(expected).await.unwrap();

            // First recv transitions from Pending to Ready
            let status1 = rx.recv().await.unwrap();
            assert_eq!(status1, expected);
            assert!(matches!(rx, WaitRx::Ready(_)));

            // Second recv returns the same status immediately
            let status2 = rx.recv().await.unwrap();
            assert_eq!(status2, expected);
        }

        #[test_log::test(tokio::test)]
        async fn recv_on_ready_returns_immediately() {
            let status = ExitStatus {
                success: true,
                code: Some(0),
            };
            let mut rx = WaitRx::Ready(status);

            let result = rx.recv().await.unwrap();
            assert_eq!(result, status);
        }

        #[test_log::test(tokio::test)]
        async fn recv_returns_error_when_sender_dropped() {
            let (tx, mut rx) = channel();
            drop(tx);

            let result = rx.recv().await;
            assert!(result.is_err());
            assert!(matches!(rx, WaitRx::Dropped));
        }

        #[test_log::test(tokio::test)]
        async fn recv_on_dropped_returns_error() {
            let mut rx = WaitRx::Dropped;
            let result = rx.recv().await;
            assert!(result.is_err());
            assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("Internal resolver dropped")
            );
        }

        #[test_log::test(tokio::test)]
        async fn send_after_rx_dropped_transitions_to_done_but_returns_error() {
            let (mut tx, rx) = channel();
            drop(rx);

            let result = tx.send(ExitStatus::killed()).await;
            // tx transitions to Done regardless of whether the receiver got it
            assert!(matches!(tx, WaitTx::Done));
            // But the send itself should fail since receiver is gone
            assert!(result.is_err());
        }
    }
}
