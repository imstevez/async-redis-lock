//! Error types returned by this crate.

use thiserror::Error as ThisError;

/// The result type used by this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur while acquiring, extending or releasing a lock.
#[derive(ThisError, Debug, PartialEq)]
#[non_exhaustive]
pub enum Error {
    /// The lock id was not found in Redis.
    ///
    /// This means the lock expired, was released, or (for multi-key locks) at
    /// least one key is no longer owned by this lock.
    #[error("lock id not found (the lock has expired or is no longer owned)")]
    IdNotFound,

    /// The configured acquisition timeout elapsed before the lock was acquired.
    #[error("timed out while acquiring the lock")]
    Timeout,

    /// A Redis extension or release command did not finish within the
    /// configured [`Options::operation_timeout`](crate::Options::operation_timeout).
    ///
    /// The command may still have been applied by Redis. The owner-token check
    /// makes retries safe, and the TTL is the final fallback.
    #[error("redis operation timed out")]
    OperationTimeout,

    /// No keys were provided to [`Locker::acquire`](crate::Locker::acquire).
    #[error("at least one lock key is required")]
    NoKeys,

    /// The supplied [`Options`](crate::Options) are invalid.
    #[error("invalid lock options: {0}")]
    InvalidOptions(&'static str),

    /// An error returned by Redis.
    #[error("redis error: {0}")]
    Redis(#[from] redis::RedisError),
}
