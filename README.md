# async-redis-lock

[![Crates.io](https://img.shields.io/crates/v/async-redis-lock)](https://crates.io/crates/async-redis-lock)
[![docs](https://img.shields.io/crates/v/async-redis-lock?color=orange&label=docs)](https://docs.rs/async-redis-lock)

A simple and easy-to-use asynchronous redis distributed lock implementation based on tokio and redis-rs.

## Key Features

- ✨ **Auto Extension** - Automatically extends lock lifetime in background until released
- 🔒 **Passive Release** - Lock automatically releases when lifetime expires after process crash
- 🎯 **Drop Support** - Supports both implicit release via drop and explicit release via method call
- 🔗 **Multi-key Locking** - Ability to lock multiple keys simultaneously ensuring atomic operations across them

## Quick Start

### Installation

```toml
[dependencies]
async-redis-lock = "0.2.1"
```

### Basic Usage

```rust
use async_redis_lock::Locker;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create locker
    let mut locker = Locker::from_redis_url("redis://127.0.0.1:6379/0").await?;

    // Acquire lock
    let lock = locker.acquire("lock_key_1", "lock_key_2").await?;

    // At this point:
    // 1. Lock is held
    // 2. Background task automatically extends lock TTL
    // 3. Safe to perform critical operations
    // ...    

    // Release lock explicitly
    // Alternative: drop(lock) for implicit release
    lock.release()?;

    Ok(())
}
```

### Automatic Release

```rust
use async_redis_lock::Locker;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut locker = Locker::from_redis_url("redis://127.0.0.1:6379/0").await?;

    // Use block scope to control lock lifetime
    {
        // Acquire lock and store in _lock variable
        // The _ prefix indicates we only care about its Drop behavior
        let _lock = locker.acquire("lock_key_1, lock_key_2").await?;
        // Perform operations that require locking
        // ...
        // Lock will be automatically released when block ends
        // Thanks to Rust's Drop trait implementation
    }

    Ok(())
}
```

### Advanced Configuration

```rust
use async_redis_lock::Locker;
use async_redis_lock::options::Options;
use std::time::Duration;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut locker = Locker::from_redis_url("redis://127.0.0.1:6379/0").await?;

    // Build a custom lock options
    let opts = Options::new()
        // Set interval between acquisition attempts
        // Default: 100ms
        .retry(Duration::from_millis(100))
        // Set maximum time to attempt acquisition
        // Default: Some(1s)
        // Note: none means retry indefinitely
        .timeout(Some(Duration::from_secs(1)))
        // Set lock time-to-live before auto-release
        // Default: 3s
        .ttl(Duration::from_secs(3))
        // Set lock auto-extend interval
        // Default: 1s
        // Recommend: ttl/3
        .extend(Duration::from_secs(1));

    // Acquire lock with the custom options
    let _lock = locker.acquire_with_options(&opts, "lock_key_1", "lock_key_2").await?;

    // Perform operations that require locking
    // ...

    Ok(())
}
```

## Important Notes

1. Don't ignore the return value of acquire method, or the lock will release immediately
2. If the extension interval is too large, the lock extension may fail because the lock has been passively released (by
   expiration) before the extension attempt, the recommend is ttl/3
3. Lock implements Drop trait and will auto-release when out of scope

## License

MIT

Contributions and suggestions are welcome!