use super::Shutdown;
use crate::utils::Timer;
use log::*;
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
    shutdown: Shutdown,
}

impl ShutdownTimer {
    // Create the timer that will be used shutdown the server after duration elapsed. The timer is
    // not started upon creation.
    pub fn new(shutdown: Shutdown) -> Self {
        let (tx, rx) = watch::channel(());
        let timer = match shutdown {
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
                    "Server shutdown timer configured: terminate after no activity for {}s",
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

        Self {
            timer,
            watcher: ShutdownNotification(rx),
            shutdown,
        }
    }

    /// Starts or restarts the timer
    pub fn start(&mut self) {
        self.timer.start();
    }

    /// Stops the timer, only applying if the timer is `lonely`
    pub fn stop(&self) {
        match self.shutdown {
            Shutdown::Lonely(_) => self.timer.stop(),
            _ => (),
        }
    }

    /// Stops the timer completely by killing the internal callback task, meaning it can never be
    /// started again
    pub fn abort(&self) {
        self.timer.abort();
    }

    /// Clones the notification
    pub fn clone_notification(&self) -> ShutdownNotification {
        self.watcher.clone()
    }
}
