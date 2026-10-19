//! Lock acquisition and maintenance options.

use std::time::Duration;

use crate::error::{Error, Result};

const DEFAULT_RETRY: Duration = Duration::from_millis(100);
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(1000);
const DEFAULT_TTL: Duration = Duration::from_millis(3000);
const DEFAULT_EXTEND: Duration = Duration::from_millis(1000);
const DEFAULT_OPERATION_TIMEOUT: Duration = Duration::from_secs(1);
const MIN_OPERATION_TIMEOUT: Duration = Duration::from_millis(1);

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
///     .extend(Duration::from_secs(1))
///     .operation_timeout(Duration::from_millis(500));
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

    /// Maximum time a single extension or release command may take.
    ///
    /// This bounds how long a Redis outage can delay auto-extension or
    /// [`Lock::release`](crate::Lock::release). Must be at least 1ms.
    ///
    /// The effective bound is `min(operation_timeout, ttl)`, because waiting
    /// longer than the lock TTL cannot improve the outcome. For timely renewal,
    /// keep it at or below `extend`. Defaults to 1s.
    pub operation_timeout: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            retry: DEFAULT_RETRY,
            timeout: Some(DEFAULT_TIMEOUT),
            ttl: DEFAULT_TTL,
            extend: DEFAULT_EXTEND,
            operation_timeout: DEFAULT_OPERATION_TIMEOUT,
        }
    }
}

impl Options {
    /// Creates options with default values.
    ///
    /// | Field               | Default |
    /// |---------------------|---------|
    /// | `retry`             | 100ms   |
    /// | `timeout`           | 1s      |
    /// | `ttl`               | 3s      |
    /// | `extend`            | 1s      |
    /// | `operation_timeout` | 1s      |
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

    /// Sets the maximum time a single extension or release command may take.
    ///
    /// This bounds how long a Redis outage can delay auto-extension or
    /// [`Lock::release`](crate::Lock::release). Must be at least 1ms.
    ///
    /// The effective bound is `min(operation_timeout, ttl)`. For timely
    /// renewal, keep it at or below `extend`.
    #[must_use]
    pub fn operation_timeout(mut self, dur: Duration) -> Self {
        self.operation_timeout = dur;
        self
    }

    /// Returns the timeout used for extension and release commands.
    ///
    /// A maintenance command cannot usefully take longer than the lock TTL.
    pub(crate) fn effective_operation_timeout(&self) -> Duration {
        self.operation_timeout.min(self.ttl)
    }

    /// Validates the options.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidOptions`] when:
    ///
    /// - `ttl` is shorter than 1ms (Redis cannot expire a key that fast),
    /// - `retry` is zero (it would busy-loop against Redis),
    /// - `extend` is zero (the extension task would busy-loop),
    /// - `operation_timeout` is shorter than 1ms.
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
        if self.operation_timeout < MIN_OPERATION_TIMEOUT {
            return Err(Error::InvalidOptions(
                "operation timeout must be at least 1ms",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_timeout_must_be_at_least_1ms() {
        let opts = Options::new().operation_timeout(Duration::ZERO);
        assert_eq!(
            opts.validate(),
            Err(Error::InvalidOptions(
                "operation timeout must be at least 1ms"
            ))
        );
    }

    #[test]
    fn effective_operation_timeout_is_capped_by_ttl() {
        let opts = Options::new()
            .ttl(Duration::from_millis(500))
            .operation_timeout(Duration::from_secs(2));
        assert_eq!(
            opts.effective_operation_timeout(),
            Duration::from_millis(500)
        );
    }

    #[test]
    fn effective_operation_timeout_keeps_smaller_configured_value() {
        let opts = Options::new()
            .ttl(Duration::from_secs(10))
            .operation_timeout(Duration::from_millis(250));
        assert_eq!(
            opts.effective_operation_timeout(),
            Duration::from_millis(250)
        );
    }
}
