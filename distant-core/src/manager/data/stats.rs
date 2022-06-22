use serde::{Deserialize, Serialize};

/// Contains stats about a specific connection
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stats {
    /// Timestamp in milliseconds
    pub start_time: u128,

    /// Total bytes sent
    pub bytes_sent: u128,

    /// Total bytes received
    pub bytes_received: u128,

    /// Total connected clients
    pub connected_client_cnt: usize,
}

impl Stats {
    #[inline]
    pub fn bytes_received_kb(&self) -> f64 {
        bytes_to_kb(self.bytes_received)
    }

    #[inline]
    pub fn bytes_received_mb(&self) -> f64 {
        bytes_to_mb(self.bytes_received)
    }

    #[inline]
    pub fn bytes_received_gb(&self) -> f64 {
        bytes_to_gb(self.bytes_received)
    }

    #[inline]
    pub fn bytes_sent_kb(&self) -> f64 {
        bytes_to_kb(self.bytes_sent)
    }

    #[inline]
    pub fn bytes_sent_mb(&self) -> f64 {
        bytes_to_mb(self.bytes_sent)
    }

    #[inline]
    pub fn bytes_sent_gb(&self) -> f64 {
        bytes_to_gb(self.bytes_sent)
    }
}

#[inline]
const fn bytes_to_kb(cnt: u128) -> f64 {
    (cnt as f64) / 1024.0
}

#[inline]
const fn bytes_to_mb(cnt: u128) -> f64 {
    bytes_to_kb(cnt) / 1024.0
}

#[inline]
const fn bytes_to_gb(cnt: u128) -> f64 {
    bytes_to_mb(cnt) / 1024.0
}
