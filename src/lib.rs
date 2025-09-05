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
//!
//! ## Quick Start
//!
//! ### Installation
//!
//! ```toml
//! [dependencies]
//! async-redis-lock = "0.0.1"
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
//!         // Set auto-extend interval (default: 1s)
//!         // Must be shorter than lifetime to prevent unintended lock release
//!         .extend_interval(Duration::from_secs(3))
//!         // Set retry interval between acquisition attempts (default: 500ms)
//!         .retry_interval(Duration::from_secs(1))
//!         // Set maximum time to attempt acquisition (default: 1s)
//!         // None means retry indefinitely
//!         .retry_timeout(Some(Duration::from_secs(3)))
//!         // Set maximum duration before auto-release (default: 2s)
//!         .lifetime(Duration::from_secs(5));
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
//! 2. extend_interval must be less than lifetime to prevent unintended release
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

    pub async fn acquire(&mut self, lock_key: &str) -> Result<Lock> {
        self.acquire_with_options(&Options::default(), lock_key)
            .await
    }

    pub async fn acquire_with_options(&mut self, opts: &Options, lock_key: &str) -> Result<Lock> {
        let lock_id = lock(
            &mut self.conn_manager,
            lock_key,
            opts.lifetime,
            opts.retry_interval,
            opts.retry_timeout,
        )
        .await?;

        let mut conn = self.conn_manager.clone();
        let opts = opts.clone();
        let lock_key_c1 = lock_key.to_owned();
        let lock_id_c1 = lock_id.clone();
        let (stop_tx, mut stop_rx) = oneshot::channel();

        spawn(async move {
            loop {
                select! {
                    _ = &mut stop_rx => break,
                    _ = sleep(opts.extend_interval) => {
                        if let Err(e) = extend(
                            &mut conn,
                            &lock_key_c1,
                            &lock_id_c1,
                            opts.lifetime,
                        )
                        .await
                        {
                            if let Some(e) = e.downcast_ref::<Error>() {
                                if matches!(e, IdNotFound) {
                                    break;
                                }
                            }
                        }
                    },
                }
            }
        });

        let cli = self.client.clone();
        let lock_key_c2 = lock_key.to_owned();
        let lock_id_c2 = lock_id.clone();

        Ok(Lock {
            release_fn: Some(Box::new(move || -> Result<()> {
                let _ = stop_tx.send(());
                let mut conn = cli.get_connection()?;
                unlock_sync(&mut conn, &lock_key_c2, &lock_id_c2)
            })),
        })
    }
}

pub struct Lock {
    pub release_fn: Option<Box<dyn FnOnce() -> Result<()> + Send + 'static>>,
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
        let lock_key = String::from("test:test_lock_exclusive_key");

        let r = locker.acquire(&lock_key).await;
        assert!(r.is_ok(), "Should acquire a lock");

        match locker.acquire(&lock_key).await.err() {
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
            locker.acquire(&lock_key).await.is_ok(),
            "Should acquire a lock after another lock is released"
        );
    }

    #[tokio::test]
    async fn test_lock_drop() {
        let mut locker = Locker::from_redis_url("redis://127.0.0.1:6379/0")
            .await
            .unwrap();
        let lock_key = "test:test_lock_drop_key";

        {
            let r = locker.acquire(&lock_key).await;
            assert!(r.is_ok(), "Should acquire a lock in a scope");

            match locker.acquire(&lock_key).await.err() {
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
            locker.acquire(&lock_key).await.is_ok(),
            "Should acquire a lock out of the prev scope"
        );
    }

    #[tokio::test]
    async fn test_lock_passive_release() {
        let mut locker = Locker::from_redis_url("redis://127.0.0.1:6379/0")
            .await
            .unwrap();
        let lock_key = "test:test_lock_passive_release_key";

        let opts = Options::new()
            .lifetime(Duration::from_secs(2))
            .extend_interval(Duration::from_secs(3));
        let r = locker.acquire_with_options(&opts, &lock_key).await;
        assert!(
            r.is_ok(),
            "Should acquire a lock with customized lifetime and extend_interval, extend_interval greater than lifetime"
        );

        sleep(Duration::from_secs(3)).await;
        assert!(
            locker.acquire(&lock_key).await.is_ok(),
            "Should passively release a lock when the lifetime is reached"
        );
    }

    #[tokio::test]
    async fn test_lock_extend() {
        let mut locker = Locker::from_redis_url("redis://127.0.0.1:6379/0")
            .await
            .unwrap();
        let lock_key = "test:test_lock_extend_key";
        let opts = Options::new()
            .lifetime(Duration::from_secs(3))
            .extend_interval(Duration::from_secs(2));
        let r = locker.acquire_with_options(&opts, &lock_key).await;
        assert!(
            r.is_ok(),
            "Should acquire a lock with customized lifetime and extend_interval, extend_interval smaller than lifetime"
        );

        sleep(Duration::from_secs(5)).await;
        match locker.acquire(&lock_key).await.err() {
            None => assert!(false, "Should extend lock lifetime automatically"),
            Some(e) => {
                assert_eq!(e.downcast_ref::<Error>().unwrap(), &Error::Timeout)
            }
        }
    }
}
