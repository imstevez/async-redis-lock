# async-redis-lock

[![Crates.io](https://img.shields.io/crates/v/async-redis-lock)](https://crates.io/crates/redis-lock)
[![docs](https://img.shields.io/crates/v/async-redis-lock?color=yellow&label=docs)](https://docs.rs/redis-lock)

A simple and easy-to-use asynchronous redis distributed lock implementation based on tokio and redis-rs.

## Key Features

- ✨ **Auto Extension** - Automatically extends lock lifetime in background until released
- 🔒 **Passive Release** - Lock automatically releases when lifetime expires after process crash
- 🎯 **Drop Support** - Supports both implicit release via drop and explicit release via method call

## Quick Start

### Installation

```toml
[dependencies]
async-redis-lock = "0.0.1"
```

### Basic Usage

```rust
use async_redis_lock::Locker;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create locker
    let mut locker = Locker::from_redis_url("redis://127.0.0.1:6379/0").await?;

    // Acquire lock
    let lock = locker.acquire("lock_key").await?;

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
        let _lock = locker.acquire("lock_key").await?;
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
        // Set auto-extend interval (default: 1s)
        // Must be shorter than lifetime to prevent unintended lock release
        .extend_interval(Duration::from_secs(3))
        // Set retry interval between acquisition attempts (default: 500ms)
        .retry_interval(Duration::from_secs(1))
        // Set maximum time to attempt acquisition (default: 1s)
        // None means retry indefinitely
        .retry_timeout(Some(Duration::from_secs(3)))
        // Set maximum duration before auto-release (default: 2s)
        .lifetime(Duration::from_secs(5));

    // Acquire lock with the custom options
    let _lock = locker.acquire_with_options(&opts, "lock_key").await?;

    // Perform operations that require locking
    // ...
    
    Ok(())
}
```

## Important Notes

1. Don't ignore the return value of acquire method, or the lock will release immediately
2. extend_interval must be less than lifetime to prevent unintended release
3. Lock implements Drop trait and will auto-release when out of scope

## License

MIT

Contributions and suggestions are welcome!