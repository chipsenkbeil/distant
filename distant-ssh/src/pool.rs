use std::io;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Weak};

use log::*;
use russh::Channel;
use russh::client::{Handle, Msg};
use russh_sftp::client::SftpSession;
use tokio::sync::Mutex;

use crate::ClientHandler;
use crate::utils::SSH_TIMEOUT_SECS;

/// Maximum number of eviction-retry attempts when `channel_open_session` fails.
const MAX_EVICT_RETRIES: usize = 5;

/// Base backoff delay between eviction retries (multiplied by attempt number).
const EVICT_BACKOFF_MS: u64 = 50;

/// Inner pool state protected by a mutex.
struct PoolInner {
    /// Cached SFTP session (evictable to free a channel slot).
    sftp: Option<Arc<SftpSession>>,
    /// Total open channels (SFTP + transient exec).
    open_count: usize,
}

/// A channel pool that manages SSH channel allocation with reactive eviction.
///
/// When `MaxSessions` is reached, the pool evicts the cached SFTP session
/// to make room for new channels. For servers with `MaxSessions > 1`, the
/// first open always succeeds — zero overhead.
pub struct ChannelPool {
    handle: Handle<ClientHandler>,
    inner: Mutex<PoolInner>,
}

impl ChannelPool {
    pub fn new(handle: Handle<ClientHandler>) -> Arc<Self> {
        Arc::new(Self {
            handle,
            inner: Mutex::new(PoolInner {
                sftp: None,
                open_count: 0,
            }),
        })
    }

    /// Get or create the cached SFTP session.
    pub async fn sftp(self: &Arc<Self>) -> io::Result<PooledSftp> {
        {
            let inner = self.inner.lock().await;
            if let Some(session) = &inner.sftp {
                return Ok(PooledSftp {
                    session: Some(Arc::clone(session)),
                    pool: Arc::downgrade(self),
                });
            }
        }

        let channel = self.open_channel().await?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(io::Error::other)?;

        let session = Arc::new(
            SftpSession::new_opts(channel.into_stream(), Some(SSH_TIMEOUT_SECS))
                .await
                .map_err(io::Error::other)?,
        );

        {
            let mut inner = self.inner.lock().await;
            inner.sftp = Some(Arc::clone(&session));
        }

        Ok(PooledSftp {
            session: Some(session),
            pool: Arc::downgrade(self),
        })
    }

    /// Open a transient exec channel.
    pub async fn open_exec(self: &Arc<Self>) -> io::Result<PooledExec> {
        let channel = self.open_channel().await?;
        Ok(PooledExec {
            channel: Some(channel),
            pool: Arc::downgrade(self),
        })
    }

    /// Open `N` exec channels sequentially, returning a fixed-size array.
    ///
    /// On partial failure, all successfully-opened channels are dropped
    /// (RAII cleanup via `PooledExec::drop`), and the error is returned.
    ///
    /// # Errors
    ///
    /// Returns an error if any channel fails to open. On partial failure,
    /// all successfully-opened channels are cleaned up before the error
    /// is returned.
    pub async fn open_execs<const N: usize>(self: &Arc<Self>) -> io::Result<[PooledExec; N]> {
        let mut opened: Vec<PooledExec> = Vec::with_capacity(N);
        for _ in 0..N {
            match self.open_exec().await {
                Ok(exec) => opened.push(exec),
                Err(e) => {
                    drop(opened);
                    return Err(e);
                }
            }
        }
        // The loop above pushes exactly N elements, so Vec length == N is guaranteed.
        Ok(opened.try_into().unwrap_or_else(|_| unreachable!()))
    }

    /// Open a channel with reactive eviction on failure.
    async fn open_channel(&self) -> io::Result<Channel<Msg>> {
        for attempt in 0..=MAX_EVICT_RETRIES {
            match self.handle.channel_open_session().await {
                Ok(channel) => {
                    let mut inner = self.inner.lock().await;
                    inner.open_count += 1;
                    return Ok(channel);
                }
                Err(e) => {
                    if attempt == MAX_EVICT_RETRIES {
                        return Err(io::Error::other(format!(
                            "Failed to open channel after {MAX_EVICT_RETRIES} eviction attempts: {e}",
                        )));
                    }

                    let evicted = self.evict_sftp().await;
                    if !evicted {
                        return Err(io::Error::other(format!(
                            "Failed to open channel (no evictable entries): {e}"
                        )));
                    }

                    let delay = EVICT_BACKOFF_MS * (attempt as u64 + 1);
                    debug!(
                        "Channel open failed, evicted SFTP session. Retrying in {delay}ms (attempt {})",
                        attempt + 1
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                }
            }
        }
        unreachable!()
    }

    /// Evict the cached SFTP session to free a channel slot.
    /// Returns true if a session was evicted.
    async fn evict_sftp(&self) -> bool {
        let mut inner = self.inner.lock().await;
        if inner.sftp.take().is_some() {
            inner.open_count = inner.open_count.saturating_sub(1);
            debug!("Evicted SFTP session to free channel slot");
            true
        } else {
            false
        }
    }

    /// Return the SFTP session to the pool cache.
    async fn return_sftp(&self, session: Arc<SftpSession>) {
        let mut inner = self.inner.lock().await;
        inner.sftp = Some(session);
    }

    /// Decrement the open channel count.
    async fn release_slot(&self) {
        let mut inner = self.inner.lock().await;
        inner.open_count = inner.open_count.saturating_sub(1);
    }
}

/// RAII guard for an SFTP session. Derefs to `SftpSession`.
/// On Drop: returns the session to the pool cache.
pub struct PooledSftp {
    session: Option<Arc<SftpSession>>,
    pool: Weak<ChannelPool>,
}

impl Deref for PooledSftp {
    type Target = SftpSession;

    fn deref(&self) -> &SftpSession {
        self.session.as_ref().expect("PooledSftp used after drop")
    }
}

impl Drop for PooledSftp {
    fn drop(&mut self) {
        let session = self.session.take().expect("PooledSftp double-drop");
        if let Some(pool) = self.pool.upgrade() {
            tokio::spawn(async move {
                pool.return_sftp(session).await;
            });
        }
    }
}

/// RAII guard for an exec channel. Derefs to `Channel<Msg>`.
/// On Drop: closes the channel and decrements pool open count.
pub struct PooledExec {
    channel: Option<Channel<Msg>>,
    pool: Weak<ChannelPool>,
}

impl Deref for PooledExec {
    type Target = Channel<Msg>;
    fn deref(&self) -> &Channel<Msg> {
        self.channel.as_ref().expect("PooledExec used after drop")
    }
}

impl DerefMut for PooledExec {
    fn deref_mut(&mut self) -> &mut Channel<Msg> {
        self.channel.as_mut().expect("PooledExec used after drop")
    }
}

impl Drop for PooledExec {
    fn drop(&mut self) {
        if let Some(channel) = self.channel.take() {
            let pool = self.pool.clone();
            tokio::spawn(async move {
                let _ = channel.close().await;
                if let Some(pool) = pool.upgrade() {
                    pool.release_slot().await;
                }
            });
        }
    }
}

impl PooledExec {
    /// Extract the raw channel for ownership transfer (spawn/shell).
    /// Returns the channel and a permit that tracks the pool slot.
    /// The slot is freed when the permit is dropped.
    pub fn take(mut self) -> (Channel<Msg>, PoolPermit) {
        let channel = self
            .channel
            .take()
            .expect("PooledExec::take called after take");
        let permit = PoolPermit {
            pool: self.pool.clone(),
        };
        (channel, permit)
    }
}

/// Lightweight RAII permit tracking a pool slot.
/// Drop decrements the pool's open count.
pub struct PoolPermit {
    pool: Weak<ChannelPool>,
}

impl Drop for PoolPermit {
    fn drop(&mut self) {
        if let Some(pool) = self.pool.upgrade() {
            tokio::spawn(async move {
                pool.release_slot().await;
            });
        }
    }
}
