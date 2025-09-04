# async-redis-lock

A simple and easy-to-use asynchronous redis distributed read-write lock implementation based on tokio and redis-rs.

## Features:

1. Automatic extension: When a lock is acquired, the lifetime of the lock will be automatically extended in background
   until the lock is released.
2. Passive release: When progress exit abnormally, the lock will be automatically released the lifetime is exhausted.
3. Drop support: Lock can be released implicitly by drop as well as explicitly by call release method.

## Examples

### 1. General usage

```rust
use async_redis_lock::Locker;
use async_redis_lock::options::Options;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create locker from the redis url.
    let mut locker = Locker::from_redis_url("redis://127.0.0.1:6379/0").await?;

    // Lock key.
    let lock_key = "lock_key";

    // Acquire a lock.
    let lock = locker.acquire(lock_key).await?;

    // Do something.
    sleep(Duration::from_secs(5)).await;

    // Release the lock explicitly.
    lock.release()?;

    // Acquire another lock in a scope.
    {
        // the type of _lock has implemented Drop trait, so the _lock will be automatically released when _lock goes out the scope.
        // Note: Do not ignore the returned value of the acquire method, otherwise the lock will be released immediately.
        let _lock = locker.acquire(lock_key).await?;
    }

    // Acquire lock with custom options.
    let opts = Options::new()
        // Set the wait time for passively released the lock when the process exits abnormally.
        .lifetime(Duration::from_secs(5))
        // Set the retry interval when the lock acquisition fails due to reasons such as existing other locks.
        .retry_interval(Duration::from_secs(1))
        // Set the retry timeout, if the value is None, retry util the lock acquisition success.
        .retry_timeout(Some(Duration::from_secs(3)))
        // Set the interval for automatically extend the lock lifetime.
        // This value should always be less than lifetime,
        // otherwise the lock will be passively released when extend the lifetime.
        .extend_interval(Duration::from_secs(3));
    locker.acquire_with_options(&opts, lock_key).await?;

    Ok(())
}
```
