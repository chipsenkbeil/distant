use std::future::Future;
use std::marker::PhantomData;
use std::str::FromStr;
use std::time::Duration;
use std::{fmt, io};

use serde::de::{DeserializeOwned, Deserializer, Error as SerdeError, Visitor};
use serde::ser::Serializer;
use serde::Serialize;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub fn serialize_to_vec<T: Serialize>(value: &T) -> io::Result<Vec<u8>> {
    rmp_serde::encode::to_vec_named(value)
        .map_err(|x| io::Error::new(io::ErrorKind::InvalidData, format!("Serialize failed: {x}")))
}

pub fn deserialize_from_slice<T: DeserializeOwned>(slice: &[u8]) -> io::Result<T> {
    rmp_serde::decode::from_slice(slice).map_err(|x| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Deserialize failed: {x}"),
        )
    })
}

/// From https://docs.rs/serde_with/1.14.0/src/serde_with/rust.rs.html#90-118
pub fn deserialize_from_str<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: FromStr,
    T::Err: fmt::Display,
{
    struct Helper<S>(PhantomData<S>);

    impl<'de, S> Visitor<'de> for Helper<S>
    where
        S: FromStr,
        <S as FromStr>::Err: fmt::Display,
    {
        type Value = S;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "a string")
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: SerdeError,
        {
            value.parse::<Self::Value>().map_err(SerdeError::custom)
        }
    }

    deserializer.deserialize_str(Helper(PhantomData))
}

/// From https://docs.rs/serde_with/1.14.0/src/serde_with/rust.rs.html#121-127
pub fn serialize_to_str<T, S>(value: &T, serializer: S) -> Result<S::Ok, S::Error>
where
    T: fmt::Display,
    S: Serializer,
{
    serializer.collect_str(&value)
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

    /// Starts the timer, re-starting the countdown if already running. If the callback has already
    /// been completed, this timer will not invoke it again; however, this will start the timer
    /// itself, which will wait the duration and then fail to trigger the callback
    pub fn start(&mut self) {
        // Cancel the active timer task
        self.stop();
        self.active_timer = None;

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
    pub fn stop(&self) {
        if let Some(task) = self.active_timer.as_ref() {
            task.abort();
        }
    }

    /// Returns true if the timer is actively running
    pub fn is_running(&self) -> bool {
        self.active_timer.is_some() && !self.active_timer.as_ref().unwrap().is_finished()
    }

    /// Aborts the timer's callback task and internal task to trigger the callback, which means
    /// that the timer will never complete the callback and starting will have no effect
    pub fn abort(&self) {
        self.stop();
        self.callback.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod timer {
        use test_log::test;

        use super::*;

        #[test(tokio::test)]
        async fn should_not_invoke_callback_regardless_of_time_if_not_started() {
            let timer = Timer::new(Duration::default(), async {});

            tokio::time::sleep(Duration::from_millis(300)).await;

            assert!(
                !timer.callback.is_finished(),
                "Callback completed unexpectedly"
            );
        }

        #[test(tokio::test)]
        async fn should_not_invoke_callback_if_only_stop_called() {
            let timer = Timer::new(Duration::default(), async {});
            timer.stop();

            tokio::time::sleep(Duration::from_millis(300)).await;

            assert!(
                !timer.callback.is_finished(),
                "Callback completed unexpectedly"
            );
        }

        #[test(tokio::test)]
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

        #[test(tokio::test)]
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

        #[test(tokio::test)]
        async fn should_trigger_callback_even_if_timer_dropped() {
            let (tx, mut rx) = mpsc::channel(1);

            let mut timer = Timer::new(Duration::default(), async move {
                let _ = tx.send(()).await;
            });
            timer.start();
            drop(timer);

            tokio::time::sleep(Duration::from_millis(300)).await;

            assert!(rx.try_recv().is_ok(), "Callback not triggered");
        }
    }
}
