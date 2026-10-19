//! Lock acquisition and maintenance options.

use std::time::Duration;

use crate::error::{Error, Result};

const DEFAULT_RETRY: Duration = Duration::from_millis(100);
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(1000);
const DEFAULT_TTL: Duration = Duration::from_millis(3000);
const DEFAULT_EXTEND: Duration = Duration::from_millis(1000);

/// Options controlling lock acquisition, lifetime and automatic extension.
///
/// ```
/// use async_redis_lock::Options;
/// use std::time::Duration;
///
/// let opts = Options::new()
///     .retry(Duration::from_millis(100))
///     .timeout(Some(Duration::from_secs(1)))
///     .ttl(Duration::from_secs(3))
///     .extend(Duration::from_secs(1));
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// Interval between two acquisition attempts.
    pub retry: Duration,

    /// Maximum time spent trying to acquire a lock.
    ///
    /// `None` retries until the lock is acquired.
    pub timeout: Option<Duration>,

    /// Lock time-to-live.
    ///
    /// The lock is released automatically after this duration unless it is
    /// extended by the background task or released explicitly. Must be at
    /// least 1ms.
    pub ttl: Duration,

    /// Interval between automatic lock extensions.
    ///
    /// It is recommended to use `ttl / 3`, so that one slow or failed
    /// extension attempt does not let the lock expire. Must be greater than
    /// zero.
    pub extend: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            retry: DEFAULT_RETRY,
            timeout: Some(DEFAULT_TIMEOUT),
            ttl: DEFAULT_TTL,
            extend: DEFAULT_EXTEND,
        }
    }
}

impl Options {
    /// Creates options with default values.
    ///
    /// | Field     | Default |
    /// |-----------|---------|
    /// | `retry`   | 100ms   |
    /// | `timeout` | 1s      |
    /// | `ttl`     | 3s      |
    /// | `extend`  | 1s      |
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the interval between two acquisition attempts.
    #[must_use]
    pub fn retry(mut self, dur: Duration) -> Self {
        self.retry = dur;
        self
    }

    /// Sets the maximum time spent trying to acquire a lock.
    ///
    /// Pass `None` to retry until the lock is acquired.
    #[must_use]
    pub fn timeout(mut self, dur: Option<Duration>) -> Self {
        self.timeout = dur;
        self
    }

    /// Sets the lock time-to-live.
    #[must_use]
    pub fn ttl(mut self, dur: Duration) -> Self {
        self.ttl = dur;
        self
    }

    /// Sets the interval between automatic lock extensions.
    #[must_use]
    pub fn extend(mut self, dur: Duration) -> Self {
        self.extend = dur;
        self
    }

    /// Validates the options.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidOptions`] when:
    ///
    /// - `ttl` is shorter than 1ms (Redis cannot expire a key that fast),
    /// - `retry` is zero (it would busy-loop against Redis),
    /// - `extend` is zero (the extension task would busy-loop).
    pub fn validate(&self) -> Result<()> {
        if self.ttl.as_millis() == 0 {
            return Err(Error::InvalidOptions("ttl must be at least 1ms"));
        }
        if self.retry.is_zero() {
            return Err(Error::InvalidOptions(
                "retry interval must be greater than zero",
            ));
        }
        if self.extend.is_zero() {
            return Err(Error::InvalidOptions(
                "extend interval must be greater than zero",
            ));
        }
        Ok(())
    }
}
