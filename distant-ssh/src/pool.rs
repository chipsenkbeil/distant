use std::io;
use std::ops::{Deref, DerefMut};
use std::sync::{Arc, Weak};

use log::*;
use russh::Channel;
use russh::client::Handle;
use russh_sftp::client::SftpSession;
use tokio::sync::Mutex;

use crate::ClientHandler;
use crate::utils::SSH_TIMEOUT_SECS;

/// Maximum number of eviction-retry attempts when `channel_open_session` fails.
const MAX_EVICT_RETRIES: usize = 5;

/// Base backoff delay between eviction retries (multiplied by attempt number).
const EVICT_BACKOFF_MS: u64 = 50;

/// Type-erased named cache entry.
enum NamedEntry {
    Sftp(Arc<SftpSession>),
    Exec(Channel<russh::client::Msg>),
}

/// Inner pool state protected by a mutex.
struct PoolInner {
    /// Named entries in LRU order (index 0 = oldest).
    named: Vec<(String, NamedEntry)>,
    /// Total open channels (named + transient).
    open_count: usize,
}

/// A channel pool that manages SSH channel allocation with reactive eviction.
///
/// When `MaxSessions` is reached, the pool evicts the least-recently-used named
/// entry to make room for new channels. For servers with `MaxSessions > 1`, the
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
                named: Vec::new(),
                open_count: 0,
            }),
        })
    }

    /// Get or create a named SFTP session.
    pub async fn sftp(self: &Arc<Self>, id: &str) -> io::Result<PooledSftp> {
        {
            let mut inner = self.inner.lock().await;
            if let Some(pos) = inner.named.iter().position(|(k, _)| k == id)
                && matches!(inner.named[pos].1, NamedEntry::Sftp(_))
            {
                let (key, entry) = inner.named.remove(pos);
                let NamedEntry::Sftp(session) = entry else {
                    unreachable!();
                };
                inner
                    .named
                    .push((key, NamedEntry::Sftp(Arc::clone(&session))));
                return Ok(PooledSftp {
                    session: Some(session),
                    id: Some(id.to_string()),
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
            inner
                .named
                .push((id.to_string(), NamedEntry::Sftp(Arc::clone(&session))));
        }

        Ok(PooledSftp {
            session: Some(session),
            id: Some(id.to_string()),
            pool: Arc::downgrade(self),
        })
    }

    /// Open a transient (unnamed) exec channel.
    pub async fn open_exec(self: &Arc<Self>) -> io::Result<PooledExec> {
        let channel = self.open_channel().await?;
        Ok(PooledExec {
            channel: Some(channel),
            id: None,
            pool: Arc::downgrade(self),
        })
    }

    /// Open a channel with reactive eviction on failure.
    async fn open_channel(&self) -> io::Result<Channel<russh::client::Msg>> {
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

                    let evicted = self.evict_lru().await;
                    if !evicted {
                        return Err(io::Error::other(format!(
                            "Failed to open channel (no evictable entries): {e}"
                        )));
                    }

                    let delay = EVICT_BACKOFF_MS * (attempt as u64 + 1);
                    debug!(
                        "Channel open failed, evicted LRU entry. Retrying in {delay}ms (attempt {})",
                        attempt + 1
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                }
            }
        }
        unreachable!()
    }

    /// Evict the least-recently-used named entry. Prefers SFTP over exec.
    /// Returns true if an entry was evicted.
    async fn evict_lru(&self) -> bool {
        let mut inner = self.inner.lock().await;

        if inner.named.is_empty() {
            return false;
        }

        // Prefer evicting SFTP entries (index 0 = oldest)
        let sftp_pos = inner
            .named
            .iter()
            .position(|(_, entry)| matches!(entry, NamedEntry::Sftp(_)));

        let pos = sftp_pos.unwrap_or(0);
        let (name, entry) = inner.named.remove(pos);
        inner.open_count = inner.open_count.saturating_sub(1);

        debug!("Evicting LRU pool entry: {name}");

        match entry {
            NamedEntry::Sftp(session) => {
                // Drop the Arc — when all references are gone, the SFTP session
                // and its underlying channel will be closed asynchronously.
                drop(session);
            }
            NamedEntry::Exec(channel) => {
                // Close the exec channel in a background task.
                tokio::spawn(async move {
                    let _ = channel.close().await;
                });
            }
        }

        true
    }

    /// Return a named SFTP session to the pool cache.
    async fn return_sftp(&self, id: String, session: Arc<SftpSession>) {
        let mut inner = self.inner.lock().await;
        if !inner.named.iter().any(|(k, _)| k == &id) {
            inner.named.push((id, NamedEntry::Sftp(session)));
        }
    }

    /// Decrement the open channel count.
    async fn release_slot(&self) {
        let mut inner = self.inner.lock().await;
        inner.open_count = inner.open_count.saturating_sub(1);
    }
}

/// RAII guard for an SFTP session. Derefs to `SftpSession`.
/// On Drop: returns named sessions to pool cache, drops unnamed ones.
pub struct PooledSftp {
    session: Option<Arc<SftpSession>>,
    id: Option<String>,
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
        if let Some(id) = self.id.take()
            && let Some(pool) = self.pool.upgrade()
        {
            tokio::spawn(async move {
                pool.return_sftp(id, session).await;
            });
        }
    }
}

/// RAII guard for an exec channel. Derefs to `Channel<Msg>`.
/// On Drop: closes the channel and decrements pool open count.
pub struct PooledExec {
    channel: Option<Channel<russh::client::Msg>>,
    id: Option<String>,
    pool: Weak<ChannelPool>,
}

impl Deref for PooledExec {
    type Target = Channel<russh::client::Msg>;
    fn deref(&self) -> &Channel<russh::client::Msg> {
        self.channel.as_ref().expect("PooledExec used after drop")
    }
}

impl DerefMut for PooledExec {
    fn deref_mut(&mut self) -> &mut Channel<russh::client::Msg> {
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
    pub fn take(mut self) -> (Channel<russh::client::Msg>, PoolPermit) {
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
