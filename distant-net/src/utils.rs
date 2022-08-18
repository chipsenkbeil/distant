use serde::{de::DeserializeOwned, Serialize};
use std::{future::Future, io, time::Duration};
use tokio::{sync::mpsc, task::JoinHandle};

pub fn serialize_to_vec<T: Serialize>(value: &T) -> io::Result<Vec<u8>> {
    rmp_serde::encode::to_vec_named(value).map_err(|x| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Serialize failed: {}", x),
        )
    })
}

pub fn deserialize_from_slice<T: DeserializeOwned>(slice: &[u8]) -> io::Result<T> {
    rmp_serde::decode::from_slice(slice).map_err(|x| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Deserialize failed: {}", x),
        )
    })
}

pub(crate) struct Timer<T>
where
    T: Send + 'static,
{
    active_timer: Option<JoinHandle<()>>,
    callback: JoinHandle<T>,
    duration: Duration,
    trigger: mpsc::Sender<bool>,
}

impl<T> Timer<T>
where
    T: Send + 'static,
{
    /// Create a new callback to trigger `future` that will be executed after `duration` is
    /// exceeded. The timer is not started yet until `start` is invoked
    pub fn new<F>(duration: Duration, future: F) -> Self
    where
        F: Future<Output = T> + Send + 'static,
    {
        let (trigger, mut trigger_rx) = mpsc::channel(1);
        let callback = tokio::spawn(async move {
            trigger_rx.recv().await;
            future.await
        });

        Self {
            active_timer: None,
            callback,
            duration,
            trigger,
        }
    }

    /// Returns duration of the timer
    pub fn duration(&self) -> Duration {
        self.duration
    }

    /// Starts the timer, re-starting the countdown if already running. If the callback has already
    /// been completed, this timer will not invoke it again; however, this will start the timer
    /// itself, which will wait the duration and then fail to trigger the callback
    pub fn start(&mut self) {
        // Cancel the active timer task
        self.stop();

        // Exit early if callback completed as starting will do nothing
        if self.callback.is_finished() {
            return;
        }

        // Create a new active timer task
        let duration = self.duration;
        let trigger = self.trigger.clone();
        self.active_timer = Some(tokio::spawn(async move {
            tokio::time::sleep(duration).await;
            let _ = trigger.send(true).await;
        }));
    }

    /// Stops the timer, cancelling the internal task, but leaving the callback in place in case
    /// the timer is re-started later
    pub fn stop(&mut self) {
        // Delete the active timer task
        if let Some(task) = self.active_timer.take() {
            task.abort();
        }
    }

    /// Aborts the timer's callback task and internal task to trigger the callback, which means
    /// that the timer will never complete the callback and starting will have no effect
    pub fn abort(&self) {
        if let Some(task) = self.active_timer.as_ref() {
            task.abort();
        }

        self.callback.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod timer {
        use super::*;

        #[tokio::test]
        async fn should_not_invoke_callback_regardless_of_time_if_not_started() {
            let timer = Timer::new(Duration::default(), async {});

            tokio::time::sleep(Duration::from_millis(300)).await;

            assert!(
                !timer.callback.is_finished(),
                "Callback completed unexpectedly"
            );
        }

        #[tokio::test]
        async fn should_not_invoke_callback_if_only_stop_called() {
            let mut timer = Timer::new(Duration::default(), async {});
            timer.stop();

            tokio::time::sleep(Duration::from_millis(300)).await;

            assert!(
                !timer.callback.is_finished(),
                "Callback completed unexpectedly"
            );
        }

        #[tokio::test]
        async fn should_finish_callback_but_not_trigger_it_if_abort_called() {
            let (tx, mut rx) = mpsc::channel(1);

            let timer = Timer::new(Duration::default(), async move {
                let _ = tx.send(()).await;
            });
            timer.abort();

            tokio::time::sleep(Duration::from_millis(300)).await;

            assert!(timer.callback.is_finished(), "Callback not finished");
            assert!(rx.try_recv().is_err(), "Callback triggered unexpectedly");
        }

        #[tokio::test]
        async fn should_trigger_callback_after_time_elapses_once_started() {
            let (tx, mut rx) = mpsc::channel(1);

            let mut timer = Timer::new(Duration::default(), async move {
                let _ = tx.send(()).await;
            });
            timer.start();

            tokio::time::sleep(Duration::from_millis(300)).await;

            assert!(timer.callback.is_finished(), "Callback not finished");
            assert!(rx.try_recv().is_ok(), "Callback not triggered");
        }
    }
}
