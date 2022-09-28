use super::Shutdown;
use crate::utils::Timer;
use log::*;
use std::ops::{Deref, DerefMut};
use std::time::Duration;
use tokio::sync::watch;

/// Cloneable notification for when a [`ShutdownTimer`] has completed.
#[derive(Clone)]
pub(crate) struct ShutdownNotification(watch::Receiver<()>);

impl ShutdownNotification {
    /// Waits to receive a notification that the shutdown timer has concluded
    pub async fn wait(&mut self) {
        let _ = self.0.changed().await;
    }
}

/// Wrapper around [`Timer`] to support shutdown-specific notifications.
pub(crate) struct ShutdownTimer {
    timer: Timer<()>,
    watcher: ShutdownNotification,
}

impl ShutdownTimer {
    pub fn new(shutdown: Shutdown) -> Self {
        // Create the timer that will be used shutdown the server after duration elapsed
        let (tx, rx) = watch::channel(());

        // NOTE: We do a manual map such that the shutdown sender is not captured and dropped when
        //       there is no shutdown after configured. This is because we need the future for the
        //       shutdown receiver to last forever in the event that there is no shutdown configured,
        //       not return immediately, which is what would happen if the sender was dropped.
        #[allow(clippy::manual_map)]
        let mut timer = match shutdown {
            // Create a timer that will complete after `duration`, dropping it to ensure that it
            // will always happen no matter if stop/abort is called
            Shutdown::After(duration) => {
                info!(
                    "Server shutdown timer configured: terminate after {}s",
                    duration.as_secs_f32()
                );
                Timer::new(duration, async move {
                    let _ = tx.send(());
                })
            }

            // Create a timer that will complete after `duration`
            Shutdown::Lonely(duration) => {
                info!(
                    "Server shutdown timer configured: terminate after no activity in {}s",
                    duration.as_secs_f32()
                );
                Timer::new(duration, async move {
                    let _ = tx.send(());
                })
            }

            // Create a timer that will never complete (max timeout possible) so we hold on to the
            // sender to avoid the receiver from completing
            Shutdown::Never => {
                info!("Server shutdown timer configured: never terminate");
                Timer::new(Duration::MAX, async move {
                    let _ = tx.send(());
                })
            }
        };

        timer.start();

        Self {
            timer,
            watcher: ShutdownNotification(rx),
        }
    }

    /// Clones the notification
    pub fn clone_notification(&self) -> ShutdownNotification {
        self.watcher.clone()
    }
}

impl Deref for ShutdownTimer {
    type Target = Timer<()>;

    fn deref(&self) -> &Self::Target {
        &self.timer
    }
}

impl DerefMut for ShutdownTimer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.timer
    }
}
