#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![deny(missing_docs)]
#![deny(rustdoc::broken_intra_doc_links)]

pub mod error;
mod execs;
pub mod options;

pub use crate::error::{Error, Result};
pub use crate::options::Options;

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use redis::ToRedisArgs;
use redis::aio::{ConnectionManager, ConnectionManagerConfig};
use tokio::sync::oneshot;
use tokio::time::{Instant, sleep};

use crate::execs::{extend, lock, unlock, unlock_sync};

/// A handle to Redis used to acquire distributed locks.
///
/// `Locker` is cheap to clone: clones share the same underlying connection
/// manager. Acquisition only borrows the locker, so it can be shared through an
/// [`Arc`] or cloned into tasks.
#[derive(Clone)]
pub struct Locker {
    client: redis::Client,
    conn_manager: ConnectionManager,
}

impl Locker {
    /// Creates a locker from a Redis URL.
    ///
    /// # Errors
    ///
    /// Returns an error if the URL is invalid or the initial connection fails.
    pub async fn from_redis_url(url: &str) -> Result<Self> {
        let client = redis::Client::open(url)?;
        Self::from_client(client).await
    }

    /// Creates a locker from an existing [`redis::Client`].
    ///
    /// This is useful when the application already owns a Redis client or when
    /// a custom TLS configuration is required.
    ///
    /// # TLS
    ///
    /// TLS is not enabled by default. Enable a TLS feature such as
    /// `redis/tls-rustls` in your own manifest and pass a `rediss://` client to
    /// this method.
    ///
    /// # Errors
    ///
    /// Returns an error if the initial connection fails.
    pub async fn from_client(client: redis::Client) -> Result<Self> {
        let config = ConnectionManagerConfig::default().set_max_delay(Duration::from_millis(2000));
        let conn_manager = ConnectionManager::new_with_config(client.clone(), config).await?;
        Ok(Self {
            client,
            conn_manager,
        })
    }

    /// Acquires a lock for one or more keys using the default [`Options`].
    ///
    /// `lock_keys` accepts anything that implements [`ToRedisArgs`], so a
    /// single key, a tuple, an array/slice or a `Vec` all work:
    ///
    /// ```no_run
    /// # use async_redis_lock::{Locker, Result};
    /// # async fn example(locker: &Locker) -> Result<()> {
    /// let _lock = locker.acquire("key").await?;
    /// let _lock = locker.acquire(("key1", "key2")).await?;
    /// let _lock = locker.acquire(&["key1", "key2"]).await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// The returned [`Lock`] releases the lock when dropped, but dropping is
    /// best-effort. Call [`Lock::release`] when the release must be awaited.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Timeout`] if the lock could not be acquired within the
    /// configured timeout, and [`Error::NoKeys`] if `lock_keys` is empty.
    #[must_use = "dropping the returned Lock immediately releases it"]
    pub async fn acquire<K: ToRedisArgs>(&self, lock_keys: K) -> Result<Lock> {
        self.acquire_with_options(&Options::default(), lock_keys)
            .await
    }

    /// Acquires a lock for one or more keys with custom [`Options`].
    ///
    /// The lock is acquired atomically: for a multi-key lock either all keys
    /// are acquired or none of them are acquired.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidOptions`] if `opts` are invalid,
    /// [`Error::NoKeys`] if `lock_keys` is empty, and [`Error::Timeout`] if the
    /// lock could not be acquired within the configured timeout.
    #[must_use = "dropping the returned Lock immediately releases it"]
    pub async fn acquire_with_options<K: ToRedisArgs>(
        &self,
        opts: &Options,
        lock_keys: K,
    ) -> Result<Lock> {
        opts.validate()?;

        let keys = lock_keys.to_redis_args();
        if keys.is_empty() {
            return Err(Error::NoKeys);
        }

        let mut conn = self.conn_manager.clone();
        let lock_id = lock(&mut conn, &keys, opts.ttl, opts.retry, opts.timeout).await?;

        let (cancel, cancel_rx) = oneshot::channel();
        let lost = Arc::new(AtomicBool::new(false));

        tokio::spawn(extend_lock(
            conn,
            keys.clone(),
            lock_id.clone(),
            opts.clone(),
            cancel_rx,
            Arc::clone(&lost),
        ));

        Ok(Lock {
            inner: Some(LockInner {
                conn: self.conn_manager.clone(),
                keys,
                lock_id,
                cancel,
            }),
            client: self.client.clone(),
            lost,
        })
    }
}

/// Extends the lock until it is released, cancelled, or lost.
async fn extend_lock(
    mut conn: ConnectionManager,
    keys: Vec<Vec<u8>>,
    lock_id: String,
    opts: Options,
    mut cancel: oneshot::Receiver<()>,
    lost: Arc<AtomicBool>,
) {
    let mut last_success = Instant::now();

    loop {
        tokio::select! {
            _ = &mut cancel => break,
            _ = sleep(opts.extend) => {
                match extend(&mut conn, &keys, &lock_id, opts.ttl).await {
                    Ok(()) => last_success = Instant::now(),
                    Err(Error::IdNotFound) => {
                        // Another owner took the lock (or it expired).
                        lost.store(true, Ordering::SeqCst);
                        break;
                    }
                    Err(_) if last_success.elapsed() >= opts.ttl => {
                        // Extension kept failing for at least one full TTL, so
                        // the lock has certainly expired.
                        lost.store(true, Ordering::SeqCst);
                        break;
                    }
                    Err(_) => {}
                }
            }
        }
    }
}

/// A held lock.
///
/// Dropping a `Lock` tries to release it in the background. Use
/// [`Lock::release`] when the release must be awaited and its result observed.
#[must_use = "the lock is released when dropped"]
pub struct Lock {
    inner: Option<LockInner>,
    client: redis::Client,
    lost: Arc<AtomicBool>,
}

struct LockInner {
    conn: ConnectionManager,
    keys: Vec<Vec<u8>>,
    lock_id: String,
    cancel: oneshot::Sender<()>,
}

impl Lock {
    /// Releases the lock and waits for Redis to confirm it.
    ///
    /// This consumes the `Lock`, so the [`Drop`] implementation becomes a
    /// no-op afterwards.
    ///
    /// # Errors
    ///
    /// Returns [`Error::IdNotFound`] when the lock had already expired or at
    /// least one key was no longer owned by this lock. Keys still owned by this
    /// lock are deleted in that case.
    pub async fn release(mut self) -> Result<()> {
        let Some(inner) = self.inner.take() else {
            return Ok(());
        };

        let LockInner {
            mut conn,
            keys,
            lock_id,
            cancel,
        } = inner;

        let _ = cancel.send(());
        let result = unlock(&mut conn, &keys, &lock_id).await;

        if matches!(&result, Err(Error::IdNotFound)) {
            self.lost.store(true, Ordering::SeqCst);
        }

        result
    }

    /// Returns `true` if the lock was lost.
    ///
    /// A lock is lost when the auto-extension task finds that the lock key is
    /// gone or is owned by somebody else, or when extension keeps failing for
    /// at least one full TTL. Operations protected by the lock should check
    /// this value before committing results, ideally together with a fencing
    /// token.
    ///
    /// The flag is updated by the extension task, so it is not instantaneous:
    /// poll it after the critical section rather than expecting a notification.
    pub fn is_lost(&self) -> bool {
        self.lost.load(Ordering::SeqCst)
    }
}

impl fmt::Debug for Lock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut debug = f.debug_struct("Lock");
        if let Some(inner) = &self.inner {
            debug
                .field("lock_id", &inner.lock_id)
                .field("key_count", &inner.keys.len())
                .field("lost", &self.is_lost());
        } else {
            debug.field("released", &true);
        }
        debug.finish()
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let Some(inner) = self.inner.take() else {
            return;
        };

        let LockInner {
            mut conn,
            keys,
            lock_id,
            cancel,
        } = inner;
        let _ = cancel.send(());

        // `Drop` cannot await. Release in the background when a runtime is
        // available, and fall back to a blocking connection otherwise. If the
        // process crashes before either path finishes, the TTL still releases
        // the lock.
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    let _ = unlock(&mut conn, &keys, &lock_id).await;
                });
            }
            Err(_) => {
                if let Ok(mut sync_conn) = self.client.get_connection() {
                    let _ = unlock_sync(&mut sync_conn, &keys, &lock_id);
                }
            }
        }
    }
}
