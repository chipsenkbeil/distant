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
            if let Some(remaining) = retries_remaining.as_mut() {
                if *remaining > 0 {
                    *remaining -= 1;
                }
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
                Duration::from_millis(if next_millis > (std::u64::MAX as f64) {
                    std::u64::MAX
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
