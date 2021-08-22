use futures::future::OptionFuture;
use log::*;
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    sync::Mutex,
    task::{JoinError, JoinHandle},
    time::{self, Instant},
};

/// Task to keep track of a possible server shutdown based on connections
pub struct ShutdownTask {
    task: JoinHandle<()>,
    tracker: Arc<Mutex<ConnTracker>>,
}

impl Future for ShutdownTask {
    type Output = Result<(), JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.task).poll(cx)
    }
}

impl ShutdownTask {
    /// Given an optional timeout, will either create the shutdown task or not,
    /// returning an optional future for the completion of the shutdown task
    /// alongside an optional connection tracker
    pub fn maybe_initialize(
        duration: Option<Duration>,
    ) -> (OptionFuture<ShutdownTask>, Option<Arc<Mutex<ConnTracker>>>) {
        match duration {
            Some(duration) => {
                let task = Self::initialize(duration);
                let tracker = task.tracker();
                let task: OptionFuture<_> = Some(task).into();
                (task, Some(tracker))
            }
            None => (None.into(), None),
        }
    }

    /// Spawns a new task that continues to monitor the time since a
    /// connection on the server existed, reporting a shutdown to all listeners
    /// once the timeout is exceeded
    pub fn initialize(duration: Duration) -> Self {
        let tracker = Arc::new(Mutex::new(ConnTracker::new()));

        let tracker_2 = Arc::clone(&tracker);
        let task = tokio::spawn(async move {
            loop {
                // Get the time since the last connection joined/left
                let (base_time, cnt) = tracker_2.lock().await.time_and_cnt();

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
                    if !tracker_2.lock().await.has_reached_timeout(duration) {
                        continue;
                    }

                    // Otherwise, we now should exit, which we do by reporting
                    debug!(
                        "Shutdown time of {}s has been reached!",
                        duration.as_secs_f32()
                    );
                    break;
                }

                // Otherwise, we just wait the full duration as worst case
                // we'll have waited just about the time desired if right
                // after waiting starts the last connection is closed
                time::sleep(duration).await;
            }
        });

        Self { task, tracker }
    }

    /// Produces a new copy of the connection tracker associated with the shutdown manager
    pub fn tracker(&self) -> Arc<Mutex<ConnTracker>> {
        Arc::clone(&self.tracker)
    }
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

    fn time_and_cnt(&self) -> (Instant, usize) {
        (self.time, self.cnt)
    }

    fn has_reached_timeout(&self, duration: Duration) -> bool {
        self.cnt == 0 && self.time.elapsed() >= duration
    }
}

#[cfg(test)]
mod tsets {
    use super::*;
    use std::thread;

    #[tokio::test]
    async fn shutdown_task_should_not_resolve_if_has_connection_regardless_of_time() {
        let mut task = ShutdownTask::initialize(Duration::from_millis(10));
        task.tracker().lock().await.increment();
        assert!(
            futures::poll!(&mut task).is_pending(),
            "Shutdown task unexpectedly completed"
        );

        time::sleep(Duration::from_millis(15)).await;

        assert!(
            futures::poll!(task).is_pending(),
            "Shutdown task unexpectedly completed"
        );
    }

    #[tokio::test]
    async fn shutdown_task_should_resolve_if_no_connection_for_minimum_duration() {
        let mut task = ShutdownTask::initialize(Duration::from_millis(10));
        assert!(
            futures::poll!(&mut task).is_pending(),
            "Shutdown task unexpectedly completed"
        );

        time::sleep(Duration::from_millis(15)).await;

        assert!(
            futures::poll!(task).is_ready(),
            "Shutdown task unexpectedly pending"
        );
    }

    #[tokio::test]
    async fn shutdown_task_should_resolve_if_no_connection_for_minimum_duration_after_connection_removed(
    ) {
        let mut task = ShutdownTask::initialize(Duration::from_millis(10));
        task.tracker().lock().await.increment();
        assert!(
            futures::poll!(&mut task).is_pending(),
            "Shutdown task unexpectedly completed"
        );

        time::sleep(Duration::from_millis(15)).await;
        assert!(
            futures::poll!(&mut task).is_pending(),
            "Shutdown task unexpectedly completed"
        );

        task.tracker().lock().await.decrement();
        time::sleep(Duration::from_millis(15)).await;

        assert!(
            futures::poll!(task).is_ready(),
            "Shutdown task unexpectedly pending"
        );
    }

    #[tokio::test]
    async fn shutdown_task_should_not_resolve_before_minimum_duration() {
        let mut task = ShutdownTask::initialize(Duration::from_millis(10));
        assert!(
            futures::poll!(&mut task).is_pending(),
            "Shutdown task unexpectedly completed"
        );

        time::sleep(Duration::from_millis(5)).await;

        assert!(
            futures::poll!(task).is_pending(),
            "Shutdown task unexpectedly completed"
        );
    }

    #[test]
    fn conn_tracker_should_update_time_when_incremented() {
        let mut tracker = ConnTracker::new();
        let (old_time, cnt) = tracker.time_and_cnt();
        assert_eq!(cnt, 0);

        // Wait to ensure that the new time will be different
        thread::sleep(Duration::from_millis(1));

        tracker.increment();
        let (new_time, cnt) = tracker.time_and_cnt();
        assert_eq!(cnt, 1);
        assert!(new_time > old_time);
    }

    #[test]
    fn conn_tracker_should_update_time_when_decremented() {
        let mut tracker = ConnTracker::new();
        tracker.increment();

        let (old_time, cnt) = tracker.time_and_cnt();
        assert_eq!(cnt, 1);

        // Wait to ensure that the new time will be different
        thread::sleep(Duration::from_millis(1));

        tracker.decrement();
        let (new_time, cnt) = tracker.time_and_cnt();
        assert_eq!(cnt, 0);
        assert!(new_time > old_time);
    }

    #[test]
    fn conn_tracker_should_not_update_time_when_decremented_if_at_zero_already() {
        let mut tracker = ConnTracker::new();
        let (old_time, cnt) = tracker.time_and_cnt();
        assert_eq!(cnt, 0);

        // Wait to ensure that the new time would be different if updated
        thread::sleep(Duration::from_millis(1));

        tracker.decrement();
        let (new_time, cnt) = tracker.time_and_cnt();
        assert_eq!(cnt, 0);
        assert!(new_time == old_time);
    }

    #[test]
    fn conn_tracker_should_report_timeout_reached_when_time_has_elapsed_and_no_connections() {
        let tracker = ConnTracker::new();
        let (_, cnt) = tracker.time_and_cnt();
        assert_eq!(cnt, 0);

        // Wait to ensure that the new time would be different if updated
        thread::sleep(Duration::from_millis(1));

        assert!(tracker.has_reached_timeout(Duration::from_millis(1)));
    }

    #[test]
    fn conn_tracker_should_not_report_timeout_reached_when_time_has_elapsed_but_has_connections() {
        let mut tracker = ConnTracker::new();
        tracker.increment();

        let (_, cnt) = tracker.time_and_cnt();
        assert_eq!(cnt, 1);

        // Wait to ensure that the new time would be different if updated
        thread::sleep(Duration::from_millis(1));

        assert!(!tracker.has_reached_timeout(Duration::from_millis(1)));
    }
}
