# async-redis-lock

[![Crates.io](https://img.shields.io/crates/v/async-redis-lock)](https://crates.io/crates/async-redis-lock)
[![docs](https://img.shields.io/crates/v/async-redis-lock?color=orange&label=docs)](https://docs.rs/async-redis-lock)
[![CI](https://github.com/imstevez/async-redis-lock/actions/workflows/ci.yml/badge.svg)](https://github.com/imstevez/async-redis-lock/actions/workflows/ci.yml)
[![MSRV](https://img.shields.io/badge/MSRV-1.89-blue)](https://github.com/imstevez/async-redis-lock)
[![License](https://img.shields.io/crates/l/async-redis-lock)](https://github.com/imstevez/async-redis-lock/blob/main/LICENSE)

A small, asynchronous Redis distributed lock built on [tokio](https://tokio.rs)
and [redis-rs](https://github.com/redis-rs/redis-rs).

`async-redis-lock` returns a `Lock` guard that is acquired atomically, kept
alive by a background TTL-extension task, and released explicitly or when
dropped.

## Features

- **Auto extension** — a background task refreshes the lock TTL while the lock is held.
- **Passive release** — the lock expires automatically if the process crashes.
- **Drop support** — dropping a `Lock` releases it on a best-effort basis.
- **Multi-key locking** — several keys are acquired atomically with a single Lua script.
- **Lost-lock detection** — `Lock::is_lost` reports when extension finds that the lock is gone.
- **Typed errors** — `Timeout`, `IdNotFound`, `NoKeys`, `InvalidOptions` and Redis errors.

## How it works

```text
acquire ──► LOCK (Lua)
              EXISTS all keys → SET all keys = <uuid> PX ttl
              │
              ├──► spawn extension task
              │      every `extend`: EXTEND (Lua)
              │      GET all keys == <uuid> → PEXPIRE all keys
              │
              └──► return Lock { keys, <uuid>, cancel }

release().await ──► UNLOCK (Lua): GET == <uuid> → DEL (only owned keys)
drop(Lock)      ──► UNLOCK scheduled in the background (best effort)
```

- Every lock acquisition uses a fresh UUID as the **owner token**. The token is
  stored as the value of every key belonging to that lock.
- Acquisition, extension and release are each a **single Lua script**, so Redis
  executes the whole check-and-write sequence atomically.
- The **TTL is the crash safety net**. While the process is alive, the extension
  task refreshes the TTL; if the process dies, Redis releases the lock when the
  TTL expires.
- If extension finds that a key is gone or holds somebody else's token, the
  lock is marked as lost (`Lock::is_lost`).

## Installation

```toml
[dependencies]
async-redis-lock = "0.3.0"
```

Requires Rust 1.89 or newer.

## API overview

| Type | Purpose |
|------|---------|
| `Locker` | Entry point. Holds a `redis::Client` and a shared connection manager. Cheap to clone and safe to share between tasks. Provides `acquire` and `acquire_with_options`. |
| `Lock` | The held lock. Provides `release().await` and `is_lost()`, and releases on `Drop`. |
| `Options` | Acquisition and lifetime tuning: `retry`, `timeout`, `ttl`, `extend`. |
| `Error` / `Result` | Crate error type and result alias. |

## Quick start

```rust,no_run
use async_redis_lock::{Locker, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let locker = Locker::from_redis_url("redis://127.0.0.1:6379/0").await?;

    // Acquire the lock. Its TTL is extended in the background while it is held.
    let lock = locker.acquire("lock_key").await?;

    // Critical section: only one process at a time can be here.
    // ...

    // Release explicitly and wait for Redis to confirm it.
    lock.release().await?;

    Ok(())
}
```

## Multiple keys

All keys of a multi-key lock are acquired atomically: either every key is
acquired, or none of them is.

```rust,no_run
use async_redis_lock::{Locker, Result};

#[tokio::main]
async fn main() -> Result<()> {
    let locker = Locker::from_redis_url("redis://127.0.0.1:6379/0").await?;

    let lock = locker.acquire(&["lock_key_1", "lock_key_2"]).await?;

    // ...

    lock.release().await?;

    Ok(())
}
```

`acquire` accepts anything that implements `redis::ToRedisArgs`: a single key,
a tuple, an array, a slice or a `Vec`. Keys stored in `String`s can be passed as
`key.as_str()` or as a slice:

```rust,no_run
# use async_redis_lock::{Locker, Result};
# async fn example(locker: &Locker) -> Result<()> {
let keys = vec![String::from("lock_key_1"), String::from("lock_key_2")];
let lock = locker.acquire(&keys[..]).await?;
lock.release().await?;
# Ok(())
# }
```

## Releasing the lock

There are two ways to release a lock.

**Explicit release** waits for Redis and reports errors:

```rust,no_run
# use async_redis_lock::{Locker, Result};
# async fn example(locker: &Locker) -> Result<()> {
let lock = locker.acquire("lock_key").await?;
// ...
lock.release().await?;
# Ok(())
# }
```

**Drop** is best-effort and cannot report errors:

```rust,no_run
# use async_redis_lock::{Locker, Result};
# async fn example(locker: &Locker) -> Result<()> {
{
    // The lock is released when `_lock` goes out of scope.
    let _lock = locker.acquire("lock_key").await?;
    // ...
}
# Ok(())
# }
```

When a `Lock` is dropped inside a tokio runtime, the unlock is scheduled as a
background task. Outside a runtime it falls back to a blocking connection. If
the process dies before either path finishes, the TTL still releases the lock.
Use `release().await` when the release must be confirmed before continuing.

## Custom options

```rust,no_run
use async_redis_lock::{Locker, Options};
use std::time::Duration;

#[tokio::main]
async fn main() -> async_redis_lock::Result<()> {
    let locker = Locker::from_redis_url("redis://127.0.0.1:6379/0").await?;

    let opts = Options::new()
        // Interval between acquisition attempts. Default: 100ms.
        .retry(Duration::from_millis(100))
        // Maximum acquisition time. Default: 1s; `None` retries forever.
        .timeout(Some(Duration::from_secs(1)))
        // Lock TTL. Default: 3s.
        .ttl(Duration::from_secs(3))
        // Extension interval. Default: 1s; `ttl / 3` is recommended.
        .extend(Duration::from_secs(1));

    let lock = locker.acquire_with_options(&opts, "lock_key").await?;
    // ...
    lock.release().await?;

    Ok(())
}
```

| Field     | Default | Meaning                                              |
|-----------|---------|------------------------------------------------------|
| `retry`   | 100ms   | Interval between acquisition attempts                |
| `timeout` | 1s      | Maximum acquisition time (`None` = retry forever)    |
| `ttl`     | 3s      | Lock lifetime before Redis releases it automatically |
| `extend`  | 1s      | Interval between background TTL extensions           |

Invalid options are rejected with `Error::InvalidOptions` before contacting
Redis: `ttl` must be at least 1ms and `retry` / `extend` must be greater than
zero. An empty key list is rejected with `Error::NoKeys`.

## Detecting lost locks

Auto-extension is asynchronous, so a lock can be lost while the protected code
is still running. Check `is_lost` before committing results:

```rust,no_run
use async_redis_lock::{Locker, Options};
use std::time::Duration;

#[tokio::main]
async fn main() -> async_redis_lock::Result<()> {
    let locker = Locker::from_redis_url("redis://127.0.0.1:6379/0").await?;

    let opts = Options::new()
        .ttl(Duration::from_secs(3))
        .extend(Duration::from_secs(1));
    let lock = locker.acquire_with_options(&opts, "lock_key").await?;

    // ... do work ...

    if lock.is_lost() {
        // The lock expired or was taken over before the work finished, so the
        // critical section may not have been exclusive. Abort or compensate.
    }

    lock.release().await?;

    Ok(())
}
```

`is_lost` is updated by the extension task, so it is not instantaneous. It
becomes `true` when extension finds a missing or foreign owner token, or when
extension keeps failing for at least one full TTL.

## Error handling

```rust,no_run
use async_redis_lock::{Error, Locker};

#[tokio::main]
async fn main() -> async_redis_lock::Result<()> {
    let locker = Locker::from_redis_url("redis://127.0.0.1:6379/0").await?;

    match locker.acquire("lock_key").await {
        Ok(lock) => {
            // ... critical section ...
            lock.release().await?;
        }
        Err(Error::Timeout) => eprintln!("the lock is busy"),
        Err(Error::Redis(err)) => eprintln!("redis error: {err}"),
        Err(err) => eprintln!("could not acquire the lock: {err}"),
    }

    Ok(())
}
```

`Error` is `#[non_exhaustive]`; always include a fallback arm when matching it.

## Guarantees and limitations

| Property | Supported |
|----------|-----------|
| Atomic multi-key acquisition | Yes (single Lua script) |
| Owner-safe release and extension | Yes (UUID token check) |
| Automatic expiry after a crash | Yes (TTL) |
| Background TTL extension | Yes |
| Lost-lock detection | Yes (`is_lost`, polling) |
| Fencing tokens | No |
| Reentrancy | No |
| Fairness / FIFO ordering | No |
| Redis Cluster | No |
| Built-in TLS | Via `redis/tls-rustls` and `Locker::from_client` |

Additional limitations:

1. **Single Redis instance.** Multi-key scripts require all keys to live in the
   same instance. Master/replica failover and key eviction can break mutual
   exclusion; this crate is not a Redlock implementation.
2. **No fencing tokens.** Timeouts, GC pauses, thread starvation or a Redis
   outage longer than the TTL can let a lock expire while the protected code
   continues. For strict correctness, pair the lock with a monotonically
   increasing fencing token in your storage layer.
3. **Use `ttl / 3` for `extend`.** A larger extension interval can let the lock
   expire between two extension attempts when Redis is slow.
4. **Drop is best-effort.** Prefer `release().await` for deterministic cleanup.
5. **Cancellation of `acquire`.** If the `acquire` future is cancelled after the
   Lua script has run but before the `Lock` is returned, the keys remain in
   Redis until the TTL expires.
6. **Dedicated keys.** Lock keys are overwritten with the owner token. Use
   dedicated lock keys and never store application data under them.

## FAQ

**My lock is released immediately. Why?**

The `Lock` guard is dropped at the end of the statement if you do not bind it.
Use `let _lock = ...` or call `release().await` on the returned value.

**Why does `release()` return `IdNotFound`?**

The lock had already expired, or (for multi-key locks) at least one key was
taken over by another owner. Keys still owned by the lock are deleted anyway.

**Why is `is_lost()` still `false` after Redis went down?**

`is_lost` is driven by extension attempts. If the runtime cannot schedule the
extension task, no check happens. It also requires one full TTL of extension
failures before the lock is declared lost.

**Can I pass a `String` directly?**

Rust cannot coerce `&String` to the generic `ToRedisArgs` parameter. Pass
`key.as_str()` or a slice such as `&keys[..]`.

**Does this work with Redis Cluster or Redlock?**

No. Multi-key scripts must run on one Redis node. Single-key locking can point
at a cluster node, but the crate does not implement Redlock or cross-slot
scripting.

**Can I use TLS?**

The crate does not enable a TLS feature by default. Enable `redis/tls-rustls` in
your own manifest and build the locker from a `rediss://` client with
`Locker::from_client`.

## Development

The integration tests require a running Redis server:

```bash
REDIS_URL=redis://127.0.0.1:6379/0 cargo test
```

The URL defaults to `redis://127.0.0.1:6379/0`. Tests use unique keys, so they
can run in parallel and against a shared server.

Runnable examples:

```bash
cargo run --example basic
cargo run --example custom_options
```

Before opening a pull request, run:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
REDIS_URL=redis://127.0.0.1:6379/0 cargo test
```

## Minimum supported Rust version

Rust 1.89. The MSRV is validated by the CI toolchain and may be raised in a
minor release when a dependency requires it.

## License

MIT. See [LICENSE](https://github.com/imstevez/async-redis-lock/blob/main/LICENSE).
