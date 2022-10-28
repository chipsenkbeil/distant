use super::Reconnectable;
use std::io;
use std::time::Duration;

/// Represents the strategy to apply when attempting to reconnect the client to the server.
#[derive(Clone, Debug)]
pub enum ReconnectStrategy {
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
    /// Creates a default strategy using exponential backoff logic starting from 1 second with
    /// a factor of 2, a maximum duration of 30 seconds, a maximum retry count of 10, and a timeout
    /// of 5 minutes per attempt.
    fn default() -> Self {
        Self::ExponentialBackoff {
            base: Duration::from_millis(1000),
            factor: 2.0,
            max_duration: Some(Duration::from_secs(30)),
            max_retries: Some(10),
            timeout: Some(Duration::from_secs(60 * 5)),
        }
    }
}

impl ReconnectStrategy {
    pub async fn reconnect<T: Reconnectable>(&mut self, reconnectable: &mut T) -> io::Result<()> {
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
            if result.is_ok() {
                return Ok(());
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

    /// Returns the maximum duration between reconnect attempts, or None if there is no limit.
    pub fn max_duration(&self) -> Option<Duration> {
        match self {
            ReconnectStrategy::ExponentialBackoff { max_duration, .. } => *max_duration,
            ReconnectStrategy::FibonacciBackoff { max_duration, .. } => *max_duration,
            ReconnectStrategy::FixedInterval { .. } => None,
        }
    }

    /// Returns the maximum reconnect attempts the strategy will perform, or None if will attempt
    /// forever.
    pub fn max_retries(&self) -> Option<usize> {
        match self {
            ReconnectStrategy::ExponentialBackoff { max_retries, .. } => *max_retries,
            ReconnectStrategy::FibonacciBackoff { max_retries, .. } => *max_retries,
            ReconnectStrategy::FixedInterval { max_retries, .. } => *max_retries,
        }
    }

    /// Returns the timeout per reconnect attempt that is associated with the strategy.
    pub fn timeout(&self) -> Option<Duration> {
        match self {
            ReconnectStrategy::ExponentialBackoff { timeout, .. } => *timeout,
            ReconnectStrategy::FibonacciBackoff { timeout, .. } => *timeout,
            ReconnectStrategy::FixedInterval { timeout, .. } => *timeout,
        }
    }

    /// Returns the initial duration to sleep.
    fn initial_sleep_duration(&self) -> Duration {
        match self {
            ReconnectStrategy::ExponentialBackoff { base, .. } => *base,
            ReconnectStrategy::FibonacciBackoff { base, .. } => *base,
            ReconnectStrategy::FixedInterval { interval, .. } => *interval,
        }
    }

    /// Adjusts next sleep duration based on the strategy.
    fn adjust_sleep(&self, prev: Option<Duration>, curr: Duration) -> Duration {
        match self {
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
