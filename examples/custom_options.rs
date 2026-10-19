//! Multi-key lock with custom acquisition and extension options.
//!
//! Start a local Redis server and run:
//!
//! ```bash
//! cargo run --example custom_options
//! ```

use async_redis_lock::{Locker, Options};
use std::time::Duration;

#[tokio::main]
async fn main() -> async_redis_lock::Result<()> {
    let locker = Locker::from_redis_url("redis://127.0.0.1:6379/0").await?;

    let opts = Options::new()
        .retry(Duration::from_millis(50))
        .timeout(Some(Duration::from_secs(2)))
        .ttl(Duration::from_secs(6))
        .extend(Duration::from_secs(2));

    let keys = ["examples:multi:a", "examples:multi:b"];
    let lock = locker.acquire_with_options(&opts, &keys[..]).await?;
    println!("acquired {lock:?}");

    // ... protected work ...

    if lock.is_lost() {
        eprintln!("the lock was lost before the work finished");
    }

    lock.release().await?;
    println!("released");

    Ok(())
}
