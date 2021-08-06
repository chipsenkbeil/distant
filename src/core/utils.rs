use log::*;
use std::{
    future::Future,
    ops::{Deref, DerefMut},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io,
    runtime::Handle,
    sync::{Mutex, Notify},
    time::{self, Instant},
};

// Generates a new tenant name
pub fn new_tenant() -> String {
    format!("tenant_{}{}", rand::random::<u16>(), rand::random::<u8>())
}

// Wraps a future in a tokio timeout call, transforming the error into
// an io error
pub async fn timeout<T, F>(d: Duration, f: F) -> io::Result<T>
where
    F: Future<Output = T>,
{
    time::timeout(d, f)
        .await
        .map_err(|x| io::Error::new(io::ErrorKind::TimedOut, x))
}

pub struct ConnTracker {
    time: Instant,
    cnt: usize,
}

impl ConnTracker {
    pub fn new() -> Self {
        Self {
            time: Instant::now(),
            cnt: 0,
        }
    }

    pub fn increment(&mut self) {
        self.time = Instant::now();
        self.cnt += 1;
    }

    pub fn decrement(&mut self) {
        if self.cnt > 0 {
            self.time = Instant::now();
            self.cnt -= 1;
        }
    }

    pub fn time_and_cnt(&self) -> (Instant, usize) {
        (self.time, self.cnt)
    }

    pub fn has_exceeded_timeout(&self, duration: Duration) -> bool {
        self.cnt == 0 && self.time.elapsed() > duration
    }
}

/// Spawns a new task that continues to monitor the time since a
/// connection on the server existed, shutting down the runtime
/// if the time is exceeded
pub fn new_shutdown_task(
    handle: Handle,
    duration: Option<Duration>,
) -> (Arc<Mutex<ConnTracker>>, Arc<Notify>) {
    let ct = Arc::new(Mutex::new(ConnTracker::new()));
    let notify = Arc::new(Notify::new());

    let ct_2 = Arc::clone(&ct);
    let notify_2 = Arc::clone(&notify);
    if let Some(duration) = duration {
        handle.spawn(async move {
            loop {
                // Get the time since the last connection joined/left
                let (base_time, cnt) = ct_2.lock().await.time_and_cnt();

                // If we have no connections left, we want to wait
                // until the remaining period has passed and then
                // verify that we still have no connections
                if cnt == 0 {
                    // Get the time we should wait based on when the last connection
                    // was dropped; this closes the gap in the case where we start
                    // sometime later than exactly duration since the last check
                    let next_time = base_time + duration;
                    let wait_duration = next_time
                        .checked_duration_since(Instant::now())
                        .unwrap_or_default()
                        + Duration::from_millis(1);

                    // Wait until we've reached our desired duration since the
                    // last connection was dropped
                    time::sleep(wait_duration).await;

                    // If we do have a connection at this point, don't exit
                    if !ct_2.lock().await.has_exceeded_timeout(duration) {
                        continue;
                    }

                    // Otherwise, we now should exit, which we do by reporting
                    debug!(
                        "Shutdown time of {}s has been reached!",
                        duration.as_secs_f32()
                    );
                    notify_2.notify_one();
                    break;
                }

                // Otherwise, we just wait the full duration as worst case
                // we'll have waited just about the time desired if right
                // after waiting starts the last connection is closed
                time::sleep(duration).await;
            }
        });
    }

    (ct, notify)
}

/// Wraps a string to provide some friendly read and write methods
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct StringBuf(String);

impl StringBuf {
    pub fn new() -> Self {
        Self(String::new())
    }

    /// Consumes data within the buffer that represent full lines (end with a newline) and returns
    /// the string containing those lines.
    ///
    /// The remaining buffer contains are returned as the second part of the tuple
    pub fn into_full_lines(mut self) -> (Option<String>, StringBuf) {
        match self.rfind('\n') {
            Some(idx) => {
                let remaining = self.0.split_off(idx + 1);
                (Some(self.0), Self(remaining))
            }
            None => (None, self),
        }
    }
}

impl From<String> for StringBuf {
    fn from(x: String) -> Self {
        Self(x)
    }
}

impl From<StringBuf> for String {
    fn from(x: StringBuf) -> Self {
        x.0
    }
}

impl Deref for StringBuf {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for StringBuf {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
