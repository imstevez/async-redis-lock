//! [![Crates.io](https://img.shields.io/crates/v/async-redis-lock)](https://crates.io/crates/redis-lock)
//! [![docs](https://img.shields.io/crates/v/async-redis-lock?color=yellow&label=docs)](https://docs.rs/redis-lock)
//!
//! A simple and easy-to-use asynchronous redis distributed lock implementation based on tokio and redis-rs.
//!
//! ## Key Features
//!
//! - ✨ **Auto Extension** - Automatically extends lock lifetime in background until released
//! - 🔒 **Passive Release** - Lock automatically releases when lifetime expires after process crash
//! - 🎯 **Drop Support** - Supports both implicit release via drop and explicit release via method call
//! - 🔗 **Multi-key Locking** - Ability to lock multiple keys simultaneously ensuring atomic operations across them
//!
//! ## Quick Start
//!
//! ### Installation
//!
//! ```toml
//! [dependencies]
//! async-redis-lock = "0.2.1"
//! ```
//!
//! ### Basic Usage
//!
//! ```rust
//! use async_redis_lock::Locker;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     // Create locker
//!     let mut locker = Locker::from_redis_url("redis://127.0.0.1:6379/0").await?;
//!
//!     // Acquire lock
//!     let lock = locker.acquire("lock_key").await?;
//!
//!     // At this point:
//!     // 1. Lock is held
//!     // 2. Background task automatically extends lock TTL
//!     // 3. Safe to perform critical operations
//!     // ...
//!
//!     // Release lock explicitly
//!     // Alternative: drop(lock) for implicit release
//!     lock.release()?;
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Automatic Release
//!
//! ```rust
//! use async_redis_lock::Locker;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let mut locker = Locker::from_redis_url("redis://127.0.0.1:6379/0").await?;
//!
//!     // Use block scope to control lock lifetime
//!     {
//!         // Acquire lock and store in _lock variable
//!         // The _ prefix indicates we only care about its Drop behavior
//!         let _lock = locker.acquire("lock_key").await?;
//!         // Perform operations that require locking
//!         // ...
//!         // Lock will be automatically released when block ends
//!         // Thanks to Rust's Drop trait implementation
//!     }
//!
//!     Ok(())
//! }
//! ```
//!
//! ### Advanced Configuration
//!
//! ```rust
//! use async_redis_lock::Locker;
//! use async_redis_lock::options::Options;
//! use std::time::Duration;
//!
//! #[tokio::main]
//! async fn main() -> anyhow::Result<()> {
//!     let mut locker = Locker::from_redis_url("redis://127.0.0.1:6379/0").await?;
//!
//!     // Build a custom lock options
//!     let opts = Options::new()
//!         // Set interval between acquisition attempts
//!         // Default: 100ms
//!         .retry(Duration::from_millis(100))
//!         // Set maximum time to attempt acquisition
//!         // Default: Some(1s)
//!         // Note: none means retry indefinitely
//!         .timeout(Some(Duration::from_secs(1)))
//!         // Set lock time-to-live before auto-release
//!         // Default: 3s
//!         .ttl(Duration::from_secs(3))
//!         // Set lock auto-extend interval
//!         // Default: 1s
//!         // Recommend: ttl/3
//!         .extend(Duration::from_secs(1));
//!
//!     // Acquire lock with the custom options
//!     let _lock = locker.acquire_with_options(&opts, "lock_key").await?;
//!
//!     // Perform operations that require locking
//!     // ...
//!
//!     Ok(())
//! }
//! ```
//! ## Important Notes
//!
//! 1. Don't ignore the return value of acquire method, or the lock will release immediately
//! 2. If the extension interval is too large, the lock extension may fail because the lock has been passively released (by expiration) before the extension attempt, the recommend is ttl/3
//! 3. Lock implements Drop trait and will auto-release when out of scope
//!
//!
pub mod error;
pub mod execs;
pub mod options;

use crate::error::Error;
use crate::error::Error::IdNotFound;
use crate::execs::*;
use crate::options::Options;

use anyhow::Result;
use redis::ToRedisArgs;
use redis::aio::{ConnectionManager, ConnectionManagerConfig};
use tokio::sync::oneshot;
use tokio::time::sleep;
use tokio::{select, spawn};

#[derive(Clone)]
pub struct Locker {
    client: redis::Client,
    conn_manager: ConnectionManager,
}

impl Locker {
    pub async fn from_redis_url(url: &str) -> Result<Self> {
        let client = redis::Client::open(url)?;
        let cfg = ConnectionManagerConfig::default().set_max_delay(2000);
        let async_conn_manager = ConnectionManager::new_with_config(client.clone(), cfg).await?;
        Ok(Self {
            client,
            conn_manager: async_conn_manager,
        })
    }

    /// Acquires a lock for the given key(s).
    ///
    /// # Arguments
    /// * `lock_keys` - A single key or a collection of keys to be locked.
    pub async fn acquire<K>(&mut self, lock_keys: &K) -> Result<Lock>
    where
        K: ToRedisArgs + Clone + Send + 'static,
    {
        self.acquire_with_options(&Options::default(), lock_keys)
            .await
    }

    /// Acquires a lock for the given key(s) with custom options.
    ///
    /// # Arguments
    /// * `opts` - Options for lock acquisition and management.
    /// * `lock_keys` - A single key or a collection of keys to be locked.
    pub async fn acquire_with_options<K>(&mut self, opts: &Options, lock_keys: &K) -> Result<Lock>
    where
        K: ToRedisArgs + Clone + Send + 'static,
    {
        let lock_id = lock(
            &mut self.conn_manager,
            lock_keys,
            opts.ttl,
            opts.retry,
            opts.timeout,
        )
        .await?;

        let mut conn = self.conn_manager.clone();
        let opts = opts.clone();
        let lock_keys_copy = lock_keys.clone();
        let lock_id_copy = lock_id.clone();
        let (stop_tx, mut stop_rx) = oneshot::channel();

        spawn(async move {
            loop {
                let lock_keys = lock_keys_copy.clone();
                select! {
                    _ = &mut stop_rx => break,
                    _ = sleep(opts.extend) => {
                        if let Err(e) = extend(
                            &mut conn,
                            lock_keys,
                            &lock_id_copy,
                            opts.ttl,
                        )
                        .await
                        {
                            if let Some(IdNotFound) = e.downcast_ref::<Error>() {
                                break;
                            }
                        }
                    },
                }
            }
        });

        let cli = self.client.clone();
        let lock_keys_copy = lock_keys.clone();
        Ok(Lock {
            release_fn: Some(Box::new(move || -> Result<()> {
                let _ = stop_tx.send(());
                let mut conn = cli.get_connection()?;
                unlock_sync(&mut conn, lock_keys_copy, &lock_id)
            })),
        })
    }
}

pub struct Lock {
    release_fn: Option<Box<dyn FnOnce() -> Result<()> + Send + 'static>>,
}

impl Lock {
    pub fn release(mut self) -> Result<()> {
        self.call_release()
    }

    fn call_release(&mut self) -> Result<()> {
        match self.release_fn.take() {
            Some(release_fn) => release_fn(),
            None => Ok(()),
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = self.call_release();
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn test_lock_exclusive() {
        let mut locker = Locker::from_redis_url("redis://127.0.0.1:6379/0")
            .await
            .unwrap();

        let lock_keys = vec![
            "test:test_lock_exclusive_key_1",
            "test:test_lock_exclusive_key_2",
        ];

        let r = locker.acquire(&lock_keys).await;
        assert!(r.is_ok(), "Should acquire a lock");

        match locker.acquire(&lock_keys).await.err() {
            None => assert!(false, "Should get an error when acquiring another lock"),
            Some(e) => {
                assert_eq!(
                    e.downcast_ref::<Error>().unwrap(),
                    &Error::Timeout,
                    "Should get a timed out error when acquiring another lock"
                )
            }
        }

        assert!(r.unwrap().release().is_ok(), "Should release a lock");

        assert!(
            locker.acquire(&lock_keys).await.is_ok(),
            "Should acquire a lock after another lock is released"
        );
    }

    #[tokio::test]
    async fn test_lock_drop() {
        let mut locker = Locker::from_redis_url("redis://127.0.0.1:6379/0")
            .await
            .unwrap();

        let lock_keys = vec!["test:test_lock_drop_key_1", "test:test_lock_drop_key_2"];

        {
            let r = locker.acquire(&lock_keys).await;
            assert!(r.is_ok(), "Should acquire a lock in a scope");

            match locker.acquire(&lock_keys).await.err() {
                None => assert!(
                    false,
                    "Should get an error when acquiring another lock in a scope"
                ),
                Some(e) => {
                    assert_eq!(
                        e.downcast_ref::<Error>().unwrap(),
                        &Error::Timeout,
                        "Should get an timed out error when acquiring another lock in a scope"
                    );
                }
            }
        }

        assert!(
            locker.acquire(&lock_keys).await.is_ok(),
            "Should acquire a lock out of the prev scope"
        );
    }

    #[tokio::test]
    async fn test_lock_passive_release() {
        let mut locker = Locker::from_redis_url("redis://127.0.0.1:6379/0")
            .await
            .unwrap();

        let lock_keys = vec![
            "test:test_lock_passive_release_key_1",
            "test:test_lock_passive_release_key_2",
        ];

        let opts = Options::new()
            .ttl(Duration::from_secs(2))
            .extend(Duration::from_secs(3));
        let r = locker.acquire_with_options(&opts, &lock_keys).await;
        assert!(
            r.is_ok(),
            "Should acquire a lock with customized lifetime and extend_interval, extend_interval greater than lifetime"
        );

        sleep(Duration::from_secs(10)).await;
        assert!(
            locker.acquire(&lock_keys).await.is_ok(),
            "Should passively release a lock when the lifetime is reached"
        );
    }

    #[tokio::test]
    async fn test_lock_extend() {
        let mut locker = Locker::from_redis_url("redis://127.0.0.1:6379/0")
            .await
            .unwrap();
        let lock_keys = vec!["test:test_lock_extend_key_1", "test:test_lock_extend_key_2"];

        let opts = Options::new()
            .ttl(Duration::from_secs(3))
            .extend(Duration::from_secs(1));
        let r = locker.acquire_with_options(&opts, &lock_keys).await;
        assert!(
            r.is_ok(),
            "Should acquire a lock with customized lifetime and extend_interval, extend_interval smaller than lifetime"
        );

        sleep(Duration::from_secs(5)).await;
        match locker.acquire(&lock_keys).await.err() {
            None => assert!(false, "Should extend lock lifetime automatically"),
            Some(e) => {
                assert_eq!(e.downcast_ref::<Error>().unwrap(), &Error::Timeout)
            }
        }
    }
}
