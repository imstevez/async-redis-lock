//! Minimal example: acquire, hold and release a lock.
//!
//! Start a local Redis server and run:
//!
//! ```bash
//! cargo run --example basic
//! ```

use async_redis_lock::{Locker, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let locker = Locker::from_redis_url("redis://127.0.0.1:6379/0").await?;

    let lock = locker.acquire("examples:basic").await?;
    println!("acquired {lock:?}");

    // Simulate some work while the background task extends the TTL.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    lock.release().await?;
    println!("released");

    Ok(())
}
