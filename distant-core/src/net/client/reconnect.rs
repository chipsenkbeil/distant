use std::io;
use std::time::Duration;

use log::*;
use strum::Display;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::Reconnectable;

/// Represents a watcher over a [`ConnectionState`].
#[derive(Clone)]
pub struct ConnectionWatcher(pub(super) watch::Receiver<ConnectionState>);

impl ConnectionWatcher {
    /// Returns next [`ConnectionState`] after a change is detected, or `None` if no more changes
    /// will be detected.
    pub async fn next(&mut self) -> Option<ConnectionState> {
        self.0.changed().await.ok()?;
        Some(self.last())
    }

    /// Returns true if the connection state has changed.
    pub fn has_changed(&self) -> bool {
        self.0.has_changed().ok().unwrap_or(false)
    }

    /// Returns the last [`ConnectionState`] observed.
    pub fn last(&self) -> ConnectionState {
        *self.0.borrow()
    }

    /// Spawns a new task that continually monitors for connection state changes and invokes the
    /// function `f` whenever a new change is detected.
    pub fn on_change<F>(&self, mut f: F) -> JoinHandle<()>
    where
        F: FnMut(ConnectionState) + Send + 'static,
    {
        let rx = self.0.clone();
        tokio::spawn(async move {
            let mut watcher = Self(rx);
            while let Some(state) = watcher.next().await {
                f(state);
            }
        })
    }
}

/// Represents the state of a connection.
#[derive(Copy, Clone, Debug, Display, PartialEq, Eq)]
#[strum(serialize_all = "snake_case")]
pub enum ConnectionState {
    /// Connection is not active, but currently going through reconnection process.
    Reconnecting,

    /// Connection is active.
    Connected,

    /// Connection is not active.
    Disconnected,
}

impl ConnectionState {
    /// Returns true if reconnecting.
    pub fn is_reconnecting(&self) -> bool {
        matches!(self, Self::Reconnecting)
    }

    /// Returns true if connected.
    pub fn is_connected(&self) -> bool {
        matches!(self, Self::Connected)
    }

    /// Returns true if disconnected.
    pub fn is_disconnected(&self) -> bool {
        matches!(self, Self::Disconnected)
    }
}

/// Represents the strategy to apply when attempting to reconnect the client to the server.
#[derive(Clone, Debug)]
pub enum ReconnectStrategy {
    /// A retry strategy that will fail immediately if a reconnect is attempted.
    Fail,

    /// A retry strategy driven by exponential back-off.
    ExponentialBackoff {
        /// Represents the initial time to wait between reconnect attempts.
        base: Duration,

        /// Factor to use when modifying the retry time, used as a multiplier.
        factor: f64,

        /// Represents the maximum duration to wait between attempts. None indicates no limit.
        max_duration: Option<Duration>,

        /// Represents the maximum attempts to retry before failing. None indicates no limit.
        max_retries: Option<usize>,

        /// Represents the maximum time to wait for a reconnect attempt. None indicates no limit.
        timeout: Option<Duration>,
    },

    /// A retry strategy driven by the fibonacci series.
    FibonacciBackoff {
        /// Represents the initial time to wait between reconnect attempts.
        base: Duration,

        /// Represents the maximum duration to wait between attempts. None indicates no limit.
        max_duration: Option<Duration>,

        /// Represents the maximum attempts to retry before failing. None indicates no limit.
        max_retries: Option<usize>,

        /// Represents the maximum time to wait for a reconnect attempt. None indicates no limit.
        timeout: Option<Duration>,
    },

    /// A retry strategy driven by a fixed interval.
    FixedInterval {
        /// Represents the time between reconnect attempts.
        interval: Duration,

        /// Represents the maximum attempts to retry before failing. None indicates no limit.
        max_retries: Option<usize>,

        /// Represents the maximum time to wait for a reconnect attempt. None indicates no limit.
        timeout: Option<Duration>,
    },
}

impl Default for ReconnectStrategy {
    /// Creates a reconnect strategy that will immediately fail.
    fn default() -> Self {
        Self::Fail
    }
}

impl ReconnectStrategy {
    pub async fn reconnect<T: Reconnectable>(&mut self, reconnectable: &mut T) -> io::Result<()> {
        // If our strategy is to immediately fail, do so
        if self.is_fail() {
            return Err(io::Error::from(io::ErrorKind::ConnectionAborted));
        }

        // Keep track of last sleep length for use in adjustment
        let mut previous_sleep = None;
        let mut current_sleep = self.initial_sleep_duration();

        // Keep track of remaining retries
        let mut retries_remaining = self.max_retries();

        // Get timeout if strategy will employ one
        let timeout = self.timeout();

        // Get maximum allowed duration between attempts
        let max_duration = self.max_duration();

        // Continue trying to reconnect while we have more tries remaining, otherwise
        // we will return the last error encountered
        let mut result = Ok(());

        while retries_remaining.is_none() || retries_remaining > Some(0) {
            // Perform reconnect attempt
            result = match timeout {
                Some(timeout) => {
                    match tokio::time::timeout(timeout, reconnectable.reconnect()).await {
                        Ok(x) => x,
                        Err(x) => Err(x.into()),
                    }
                }
                None => reconnectable.reconnect().await,
            };

            // If reconnect was successful, we're done and we can exit
            match &result {
                Ok(()) => return Ok(()),
                Err(x) => {
                    error!("Failed to reconnect: {x}");
                }
            }

            // Decrement remaining retries if we have a limit
            if let Some(remaining) = retries_remaining.as_mut()
                && *remaining > 0
            {
                *remaining -= 1;
            }

            // Sleep before making next attempt
            tokio::time::sleep(current_sleep).await;

            // Update our sleep duration
            let next_sleep = self.adjust_sleep(previous_sleep, current_sleep);
            previous_sleep = Some(current_sleep);
            current_sleep = if let Some(duration) = max_duration {
                std::cmp::min(next_sleep, duration)
            } else {
                next_sleep
            };
        }

        result
    }

    /// Returns true if this strategy is the fail variant.
    pub fn is_fail(&self) -> bool {
        matches!(self, Self::Fail)
    }

    /// Returns true if this strategy is the exponential backoff variant.
    pub fn is_exponential_backoff(&self) -> bool {
        matches!(self, Self::ExponentialBackoff { .. })
    }

    /// Returns true if this strategy is the fibonacci backoff variant.
    pub fn is_fibonacci_backoff(&self) -> bool {
        matches!(self, Self::FibonacciBackoff { .. })
    }

    /// Returns true if this strategy is the fixed interval variant.
    pub fn is_fixed_interval(&self) -> bool {
        matches!(self, Self::FixedInterval { .. })
    }

    /// Returns the maximum duration between reconnect attempts, or None if there is no limit.
    pub fn max_duration(&self) -> Option<Duration> {
        match self {
            ReconnectStrategy::Fail => None,
            ReconnectStrategy::ExponentialBackoff { max_duration, .. } => *max_duration,
            ReconnectStrategy::FibonacciBackoff { max_duration, .. } => *max_duration,
            ReconnectStrategy::FixedInterval { .. } => None,
        }
    }

    /// Returns the maximum reconnect attempts the strategy will perform, or None if will attempt
    /// forever.
    pub fn max_retries(&self) -> Option<usize> {
        match self {
            ReconnectStrategy::Fail => None,
            ReconnectStrategy::ExponentialBackoff { max_retries, .. } => *max_retries,
            ReconnectStrategy::FibonacciBackoff { max_retries, .. } => *max_retries,
            ReconnectStrategy::FixedInterval { max_retries, .. } => *max_retries,
        }
    }

    /// Returns the timeout per reconnect attempt that is associated with the strategy.
    pub fn timeout(&self) -> Option<Duration> {
        match self {
            ReconnectStrategy::Fail => None,
            ReconnectStrategy::ExponentialBackoff { timeout, .. } => *timeout,
            ReconnectStrategy::FibonacciBackoff { timeout, .. } => *timeout,
            ReconnectStrategy::FixedInterval { timeout, .. } => *timeout,
        }
    }

    /// Returns the initial duration to sleep.
    fn initial_sleep_duration(&self) -> Duration {
        match self {
            ReconnectStrategy::Fail => Duration::new(0, 0),
            ReconnectStrategy::ExponentialBackoff { base, .. } => *base,
            ReconnectStrategy::FibonacciBackoff { base, .. } => *base,
            ReconnectStrategy::FixedInterval { interval, .. } => *interval,
        }
    }

    /// Adjusts next sleep duration based on the strategy.
    fn adjust_sleep(&self, prev: Option<Duration>, curr: Duration) -> Duration {
        match self {
            ReconnectStrategy::Fail => Duration::new(0, 0),
            ReconnectStrategy::ExponentialBackoff { factor, .. } => {
                let next_millis = (curr.as_millis() as f64) * factor;
                Duration::from_millis(if next_millis > (u64::MAX as f64) {
                    u64::MAX
                } else {
                    next_millis as u64
                })
            }
            ReconnectStrategy::FibonacciBackoff { .. } => {
                let prev = prev.unwrap_or_else(|| Duration::new(0, 0));
                prev.checked_add(curr).unwrap_or(Duration::MAX)
            }
            ReconnectStrategy::FixedInterval { .. } => curr,
        }
    }
}

#[cfg(test)]
mod tests {
    //! Tests for reconnection infrastructure: ConnectionState predicates, ConnectionWatcher
    //! state transitions, and ReconnectStrategy variants (Fail, ExponentialBackoff,
    //! FibonacciBackoff, FixedInterval) including timeout behavior.

    use test_log::test;

    use super::*;

    // ---------------------------------------------------------------
    // MockReconnectable for testing reconnect()
    // ---------------------------------------------------------------

    struct MockReconnectable(bool);

    impl Reconnectable for MockReconnectable {
        fn reconnect<'a>(
            &'a mut self,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<()>> + Send + 'a>>
        {
            let success = self.0;
            Box::pin(async move {
                if success {
                    Ok(())
                } else {
                    Err(io::Error::from(io::ErrorKind::ConnectionRefused))
                }
            })
        }
    }

    // ---------------------------------------------------------------
    // ConnectionState tests
    // ---------------------------------------------------------------

    #[test]
    fn connection_state_is_reconnecting() {
        assert!(ConnectionState::Reconnecting.is_reconnecting());
        assert!(!ConnectionState::Connected.is_reconnecting());
        assert!(!ConnectionState::Disconnected.is_reconnecting());
    }

    #[test]
    fn connection_state_is_connected() {
        assert!(!ConnectionState::Reconnecting.is_connected());
        assert!(ConnectionState::Connected.is_connected());
        assert!(!ConnectionState::Disconnected.is_connected());
    }

    #[test]
    fn connection_state_is_disconnected() {
        assert!(!ConnectionState::Reconnecting.is_disconnected());
        assert!(!ConnectionState::Connected.is_disconnected());
        assert!(ConnectionState::Disconnected.is_disconnected());
    }

    #[test]
    fn connection_state_display_format() {
        assert_eq!(ConnectionState::Reconnecting.to_string(), "reconnecting");
        assert_eq!(ConnectionState::Connected.to_string(), "connected");
        assert_eq!(ConnectionState::Disconnected.to_string(), "disconnected");
    }

    // ---------------------------------------------------------------
    // ConnectionWatcher tests
    // ---------------------------------------------------------------

    #[test(tokio::test)]
    async fn connection_watcher_next_returns_state_after_change() {
        let (tx, rx) = watch::channel(ConnectionState::Connected);
        let mut watcher = ConnectionWatcher(rx);

        tx.send_replace(ConnectionState::Disconnected);
        let state = watcher.next().await;
        assert_eq!(state, Some(ConnectionState::Disconnected));
    }

    #[test(tokio::test)]
    async fn connection_watcher_next_returns_none_when_sender_dropped() {
        let (tx, rx) = watch::channel(ConnectionState::Connected);
        let mut watcher = ConnectionWatcher(rx);

        drop(tx);
        let state = watcher.next().await;
        assert_eq!(state, None);
    }

    #[test]
    fn connection_watcher_has_changed_returns_correct_value() {
        let (tx, rx) = watch::channel(ConnectionState::Connected);
        let watcher = ConnectionWatcher(rx);

        // Initially no change has been detected
        assert!(!watcher.has_changed());

        // After a state update, has_changed should be true
        tx.send_replace(ConnectionState::Reconnecting);
        assert!(watcher.has_changed());
    }

    #[test]
    fn connection_watcher_has_changed_returns_false_when_sender_dropped() {
        let (tx, rx) = watch::channel(ConnectionState::Connected);
        let watcher = ConnectionWatcher(rx);
        drop(tx);

        // When sender is dropped, has_changed returns false (the ok().unwrap_or(false) path)
        assert!(!watcher.has_changed());
    }

    #[test]
    fn connection_watcher_last_returns_current_state() {
        let (tx, rx) = watch::channel(ConnectionState::Connected);
        let watcher = ConnectionWatcher(rx);

        assert_eq!(watcher.last(), ConnectionState::Connected);

        tx.send_replace(ConnectionState::Disconnected);
        assert_eq!(watcher.last(), ConnectionState::Disconnected);
    }

    #[test(tokio::test)]
    async fn connection_watcher_on_change_invokes_callback() {
        let (tx, rx) = watch::channel(ConnectionState::Connected);
        let watcher = ConnectionWatcher(rx);

        let (notify_tx, mut notify_rx) = tokio::sync::mpsc::channel(4);
        let _handle = watcher.on_change(move |state| {
            let _ = notify_tx.try_send(state);
        });

        tx.send_replace(ConnectionState::Reconnecting);
        assert_eq!(notify_rx.recv().await, Some(ConnectionState::Reconnecting));

        tx.send_replace(ConnectionState::Disconnected);
        assert_eq!(notify_rx.recv().await, Some(ConnectionState::Disconnected));

        // Drop the sender to end the watcher loop
        drop(tx);
        assert_eq!(notify_rx.recv().await, None);
    }

    // ---------------------------------------------------------------
    // ReconnectStrategy tests
    // ---------------------------------------------------------------

    #[test]
    fn reconnect_strategy_default_is_fail() {
        let strategy = ReconnectStrategy::default();
        assert!(strategy.is_fail());
    }

    #[test]
    fn reconnect_strategy_is_fail() {
        assert!(ReconnectStrategy::Fail.is_fail());
        assert!(!ReconnectStrategy::Fail.is_exponential_backoff());
        assert!(!ReconnectStrategy::Fail.is_fibonacci_backoff());
        assert!(!ReconnectStrategy::Fail.is_fixed_interval());
    }

    #[test]
    fn reconnect_strategy_is_exponential_backoff() {
        let strategy = ReconnectStrategy::ExponentialBackoff {
            base: Duration::from_millis(100),
            factor: 2.0,
            max_duration: None,
            max_retries: None,
            timeout: None,
        };
        assert!(!strategy.is_fail());
        assert!(strategy.is_exponential_backoff());
        assert!(!strategy.is_fibonacci_backoff());
        assert!(!strategy.is_fixed_interval());
    }

    #[test]
    fn reconnect_strategy_is_fibonacci_backoff() {
        let strategy = ReconnectStrategy::FibonacciBackoff {
            base: Duration::from_millis(100),
            max_duration: None,
            max_retries: None,
            timeout: None,
        };
        assert!(!strategy.is_fail());
        assert!(!strategy.is_exponential_backoff());
        assert!(strategy.is_fibonacci_backoff());
        assert!(!strategy.is_fixed_interval());
    }

    #[test]
    fn reconnect_strategy_is_fixed_interval() {
        let strategy = ReconnectStrategy::FixedInterval {
            interval: Duration::from_millis(100),
            max_retries: None,
            timeout: None,
        };
        assert!(!strategy.is_fail());
        assert!(!strategy.is_exponential_backoff());
        assert!(!strategy.is_fibonacci_backoff());
        assert!(strategy.is_fixed_interval());
    }

    #[test]
    fn reconnect_strategy_max_duration_for_each_variant() {
        assert_eq!(ReconnectStrategy::Fail.max_duration(), None);

        let max = Duration::from_secs(30);
        assert_eq!(
            ReconnectStrategy::ExponentialBackoff {
                base: Duration::from_millis(100),
                factor: 2.0,
                max_duration: Some(max),
                max_retries: None,
                timeout: None,
            }
            .max_duration(),
            Some(max)
        );

        assert_eq!(
            ReconnectStrategy::ExponentialBackoff {
                base: Duration::from_millis(100),
                factor: 2.0,
                max_duration: None,
                max_retries: None,
                timeout: None,
            }
            .max_duration(),
            None
        );

        assert_eq!(
            ReconnectStrategy::FibonacciBackoff {
                base: Duration::from_millis(100),
                max_duration: Some(max),
                max_retries: None,
                timeout: None,
            }
            .max_duration(),
            Some(max)
        );

        assert_eq!(
            ReconnectStrategy::FibonacciBackoff {
                base: Duration::from_millis(100),
                max_duration: None,
                max_retries: None,
                timeout: None,
            }
            .max_duration(),
            None
        );

        // FixedInterval always returns None for max_duration
        assert_eq!(
            ReconnectStrategy::FixedInterval {
                interval: Duration::from_millis(100),
                max_retries: None,
                timeout: None,
            }
            .max_duration(),
            None
        );
    }

    #[test]
    fn reconnect_strategy_max_retries_for_each_variant() {
        assert_eq!(ReconnectStrategy::Fail.max_retries(), None);

        assert_eq!(
            ReconnectStrategy::ExponentialBackoff {
                base: Duration::from_millis(100),
                factor: 2.0,
                max_duration: None,
                max_retries: Some(5),
                timeout: None,
            }
            .max_retries(),
            Some(5)
        );

        assert_eq!(
            ReconnectStrategy::FibonacciBackoff {
                base: Duration::from_millis(100),
                max_duration: None,
                max_retries: Some(3),
                timeout: None,
            }
            .max_retries(),
            Some(3)
        );

        assert_eq!(
            ReconnectStrategy::FixedInterval {
                interval: Duration::from_millis(100),
                max_retries: Some(10),
                timeout: None,
            }
            .max_retries(),
            Some(10)
        );

        assert_eq!(
            ReconnectStrategy::FixedInterval {
                interval: Duration::from_millis(100),
                max_retries: None,
                timeout: None,
            }
            .max_retries(),
            None
        );
    }

    #[test]
    fn reconnect_strategy_timeout_for_each_variant() {
        assert_eq!(ReconnectStrategy::Fail.timeout(), None);

        let t = Duration::from_secs(5);
        assert_eq!(
            ReconnectStrategy::ExponentialBackoff {
                base: Duration::from_millis(100),
                factor: 2.0,
                max_duration: None,
                max_retries: None,
                timeout: Some(t),
            }
            .timeout(),
            Some(t)
        );

        assert_eq!(
            ReconnectStrategy::FibonacciBackoff {
                base: Duration::from_millis(100),
                max_duration: None,
                max_retries: None,
                timeout: Some(t),
            }
            .timeout(),
            Some(t)
        );

        assert_eq!(
            ReconnectStrategy::FixedInterval {
                interval: Duration::from_millis(100),
                max_retries: None,
                timeout: Some(t),
            }
            .timeout(),
            Some(t)
        );

        assert_eq!(
            ReconnectStrategy::FixedInterval {
                interval: Duration::from_millis(100),
                max_retries: None,
                timeout: None,
            }
            .timeout(),
            None
        );
    }

    #[test(tokio::test)]
    async fn reconnect_strategy_fail_immediately_errors() {
        let mut strategy = ReconnectStrategy::Fail;
        let mut mock = MockReconnectable(true);

        let result = strategy.reconnect(&mut mock).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::ConnectionAborted);
    }

    #[test(tokio::test)]
    async fn reconnect_strategy_fixed_interval_succeeds_on_first_try() {
        let mut strategy = ReconnectStrategy::FixedInterval {
            interval: Duration::from_millis(1),
            max_retries: Some(3),
            timeout: None,
        };
        let mut mock = MockReconnectable(true);

        let result = strategy.reconnect(&mut mock).await;
        assert!(result.is_ok());
    }

    #[test(tokio::test)]
    async fn reconnect_strategy_fixed_interval_fails_after_max_retries() {
        let mut strategy = ReconnectStrategy::FixedInterval {
            interval: Duration::from_millis(1),
            max_retries: Some(2),
            timeout: None,
        };
        let mut mock = MockReconnectable(false);

        let result = strategy.reconnect(&mut mock).await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::ConnectionRefused);
    }

    #[test(tokio::test)]
    async fn reconnect_strategy_exponential_backoff_succeeds_on_first_try() {
        let mut strategy = ReconnectStrategy::ExponentialBackoff {
            base: Duration::from_millis(1),
            factor: 2.0,
            max_duration: None,
            max_retries: Some(3),
            timeout: None,
        };
        let mut mock = MockReconnectable(true);

        let result = strategy.reconnect(&mut mock).await;
        assert!(result.is_ok());
    }

    #[test(tokio::test)]
    async fn reconnect_strategy_exponential_backoff_fails_after_max_retries() {
        let mut strategy = ReconnectStrategy::ExponentialBackoff {
            base: Duration::from_millis(1),
            factor: 2.0,
            max_duration: None,
            max_retries: Some(2),
            timeout: None,
        };
        let mut mock = MockReconnectable(false);

        let result = strategy.reconnect(&mut mock).await;
        assert!(result.is_err());
    }

    #[test(tokio::test)]
    async fn reconnect_strategy_fibonacci_backoff_succeeds_on_first_try() {
        let mut strategy = ReconnectStrategy::FibonacciBackoff {
            base: Duration::from_millis(1),
            max_duration: None,
            max_retries: Some(3),
            timeout: None,
        };
        let mut mock = MockReconnectable(true);

        let result = strategy.reconnect(&mut mock).await;
        assert!(result.is_ok());
    }

    #[test(tokio::test)]
    async fn reconnect_strategy_fibonacci_backoff_fails_after_max_retries() {
        let mut strategy = ReconnectStrategy::FibonacciBackoff {
            base: Duration::from_millis(1),
            max_duration: None,
            max_retries: Some(2),
            timeout: None,
        };
        let mut mock = MockReconnectable(false);

        let result = strategy.reconnect(&mut mock).await;
        assert!(result.is_err());
    }

    #[test(tokio::test)]
    async fn reconnect_strategy_with_timeout_fails_on_slow_reconnect() {
        struct SlowReconnectable;
        impl Reconnectable for SlowReconnectable {
            fn reconnect<'a>(
                &'a mut self,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = io::Result<()>> + Send + 'a>>
            {
                Box::pin(async {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    Ok(())
                })
            }
        }

        let mut strategy = ReconnectStrategy::FixedInterval {
            interval: Duration::from_millis(1),
            max_retries: Some(1),
            timeout: Some(Duration::from_millis(10)),
        };
        let mut mock = SlowReconnectable;

        let result = strategy.reconnect(&mut mock).await;
        assert!(result.is_err());
    }
}
