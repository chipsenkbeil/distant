//! Prediction mode configuration and RTT estimation.
//!
//! Provides the [`PredictMode`] enum (used by CLI arguments) and the
//! [`RttEstimator`] (Jacobson/Karels smoothed RTT tracker used by the
//! framebuffer's prediction overlay).

use std::time::Duration;

/// Controls whether the prediction engine displays speculative local echo.
///
/// In `Adaptive` mode (the default), predictions are shown only when the
/// measured round-trip time is high enough that the user would perceive
/// latency. `On` forces predictions unconditionally, while `Off` disables
/// them entirely.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum PredictMode {
    /// Never predict — all output comes from the server.
    Off,

    /// Predict based on measured RTT. Predictions activate when SRTT
    /// exceeds 30 ms.
    #[default]
    Adaptive,

    /// Always predict — every printable keystroke is echoed locally.
    On,

    /// Always predict and skip epoch confirmation — predictions display
    /// immediately after epoch boundaries without waiting for server echo.
    Fast,

    /// Predict when SRTT exceeds 30 ms (like `Adaptive`), but once active,
    /// skip epoch confirmation (like `Fast`).
    FastAdaptive,
}

/// Jacobson/Karels smoothed RTT estimator.
///
/// Tracks a smoothed RTT (`srtt`) and RTT variance (`rttvar`) using the
/// classic TCP algorithm. Used by the prediction overlay to decide whether
/// adaptive prediction should be active.
pub struct RttEstimator {
    /// Smoothed round-trip time.
    srtt: Duration,
    /// RTT variance estimate.
    rttvar: Duration,
}

impl RttEstimator {
    /// Creates a new estimator with initial SRTT of 100 ms and variance of 50 ms.
    pub fn new() -> Self {
        Self {
            srtt: Duration::from_millis(100),
            rttvar: Duration::from_millis(50),
        }
    }

    /// Updates the estimator with a new RTT sample.
    ///
    /// Uses Jacobson/Karels algorithm with alpha = 1/8 and beta = 1/4.
    /// All arithmetic is saturating to avoid overflow panics on extreme values.
    pub fn update(&mut self, sample: Duration) {
        // rttvar = (1 - beta) * rttvar + beta * |srtt - sample|
        //        = 3/4 * rttvar + 1/4 * |srtt - sample|
        let diff = if sample > self.srtt {
            sample.saturating_sub(self.srtt)
        } else {
            self.srtt.saturating_sub(sample)
        };
        let three_quarters_var = self.rttvar.saturating_mul(3) / 4;
        let quarter_diff = diff / 4;
        self.rttvar = three_quarters_var.saturating_add(quarter_diff);

        // srtt = (1 - alpha) * srtt + alpha * sample
        //      = 7/8 * srtt + 1/8 * sample
        let seven_eighths_srtt = self.srtt.saturating_mul(7) / 8;
        let eighth_sample = sample / 8;
        self.srtt = seven_eighths_srtt.saturating_add(eighth_sample);
    }

    /// Returns the current smoothed RTT estimate.
    pub fn srtt(&self) -> Duration {
        self.srtt
    }
}

impl Default for RttEstimator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rtt_update_should_converge_toward_samples() {
        let mut rtt = RttEstimator::new();
        for _ in 0..50 {
            rtt.update(Duration::from_millis(20));
        }
        let final_srtt = rtt.srtt();
        assert!(
            final_srtt < Duration::from_millis(25),
            "SRTT should converge to ~20ms, got {:?}",
            final_srtt
        );
        assert!(
            final_srtt >= Duration::from_millis(18),
            "SRTT should not undershoot 18ms, got {:?}",
            final_srtt
        );
    }

    #[test]
    fn rtt_update_should_converge_toward_high_samples() {
        let mut rtt = RttEstimator::new();
        for _ in 0..50 {
            rtt.update(Duration::from_millis(500));
        }
        let final_srtt = rtt.srtt();
        assert!(
            final_srtt > Duration::from_millis(490),
            "SRTT should converge to ~500ms, got {:?}",
            final_srtt
        );
        assert!(
            final_srtt <= Duration::from_millis(510),
            "SRTT should not overshoot 510ms, got {:?}",
            final_srtt
        );
    }

    #[test]
    fn rtt_new_should_have_default_values() {
        let rtt = RttEstimator::new();
        assert_eq!(rtt.srtt(), Duration::from_millis(100));
    }

    #[test]
    fn rtt_default_should_match_new() {
        let rtt_new = RttEstimator::new();
        let rtt_default = RttEstimator::default();
        assert_eq!(rtt_new.srtt(), rtt_default.srtt());
    }

    #[test]
    fn rtt_single_update_should_blend_with_initial() {
        let mut rtt = RttEstimator::new();
        // srtt = 7/8 * 100 + 1/8 * 200 = 87.5 + 25 = 112.5ms
        rtt.update(Duration::from_millis(200));
        let srtt_ms = rtt.srtt().as_millis();
        assert_eq!(srtt_ms, 112, "single update should blend 7/8 old + 1/8 new");
    }

    #[test]
    fn rtt_update_with_zero_sample_should_decrease_srtt() {
        let mut rtt = RttEstimator::new();
        rtt.update(Duration::ZERO);
        assert_eq!(
            rtt.srtt().as_millis(),
            87,
            "zero sample should blend to 87ms"
        );
    }

    #[test]
    fn rtt_update_with_max_duration_should_saturate_without_overflow() {
        let mut rtt = RttEstimator::new();
        rtt.update(Duration::MAX);
        let srtt = rtt.srtt();
        assert!(
            srtt > Duration::from_secs(1_000_000),
            "SRTT should be very large after MAX sample, got {:?}",
            srtt
        );
    }

    #[test]
    fn rtt_variance_should_decrease_with_consistent_samples() {
        let mut rtt = RttEstimator::new();
        for _ in 0..50 {
            rtt.update(Duration::from_millis(50));
        }
        assert!(
            rtt.rttvar < Duration::from_millis(5),
            "variance should be small with consistent samples, got {:?}",
            rtt.rttvar
        );
    }

    #[test]
    fn rtt_variance_should_be_larger_with_variable_samples() {
        let mut rtt_consistent = RttEstimator::new();
        let mut rtt_variable = RttEstimator::new();

        for _ in 0..50 {
            rtt_consistent.update(Duration::from_millis(50));
        }

        for i in 0..50 {
            let sample = if i % 2 == 0 { 20 } else { 200 };
            rtt_variable.update(Duration::from_millis(sample));
        }

        assert!(
            rtt_variable.rttvar > rtt_consistent.rttvar,
            "variable samples should produce higher variance: consistent={:?}, variable={:?}",
            rtt_consistent.rttvar,
            rtt_variable.rttvar
        );
    }
}
