# Changelog

All notable changes to this project are documented in this file.

## 0.4.0 - 2026-10-19

### Breaking changes

- `Options` gained the public `operation_timeout` field (default 1s). Code that
  builds `Options` with a struct literal must set it or use
  `..Options::default()`.

### Added

- `Options::operation_timeout` (default 1s) bounds auto-extension and release
  commands, so a Redis outage cannot hold a maintenance command for the
  connection manager's full reconnect/retry window. The effective bound is
  `min(operation_timeout, ttl)`.
- `Error::OperationTimeout` for an extension or release command that exceeds
  that bound.

### Changed

- Lua scripts are now built once with `LazyLock<Script>` instead of being
  rebuilt, and SHA1-hashed, on every acquire, extend or release.

### Fixed

- Duplicate keys passed to `acquire` are deduplicated, so `release()` no longer
  reports `IdNotFound` after successfully deleting the key.
- If `release()` fails with a Redis error, the guard now keeps its state so
  `Drop` can make one best-effort retry instead of leaving the key to expire.
- Acquisition timeouts use `Instant::checked_add`, so an oversized timeout no
  longer panics.
- CI now validates the declared MSRV (Rust 1.89) in a dedicated job.

### Documentation

- Documented duplicate-key handling and the cleanup semantics of a failed
  `release()`.
- Documented bounded extension/release commands and their interaction with the
  lock TTL.

## 0.3.0 - 2026-10-19

### Breaking changes

- `Locker::acquire` and `Locker::acquire_with_options` now take the keys by
  value (`K: ToRedisArgs`) instead of `&K`, and borrow the locker (`&self`).
  This makes `locker.acquire("key")` and `locker.acquire(&["a", "b"])` work as
  documented. The `Clone + Send + 'static` bounds are gone.
- `Lock::release` is now `async` and awaits the reply from Redis. It reuses the
  shared connection manager instead of opening a new TCP connection.
- The public API returns `async_redis_lock::Result<T>` (`thiserror` based)
  instead of `anyhow::Result<T>`.
- The internal `execs` module is no longer public.

### Added

- `Lock::is_lost()` to detect that the lock expired or was taken over.
- `Locker::from_client()` for dependency injection and custom clients.
- Validation of `Options` and of the key list (`Error::InvalidOptions`,
  `Error::NoKeys`).
- `#[must_use]` on `Lock` and the acquire methods.
- Integration tests in `tests/lock.rs`, configurable through `REDIS_URL`.
- CI workflow running the test suite against a Redis service.

### Fixed

- README and crate-level doctest examples now compile (`cargo test` is green).
- Dropping a `Lock` no longer blocks the async runtime; the release is
  scheduled on the current runtime with a sync fallback outside of a runtime.
- Multi-key release deletes the keys that are still owned by the lock instead
  of leaving them behind when one key was lost.
- Acquisition timeouts use a monotonic clock and no longer overshoot the
  configured timeout by a full retry interval.
- Zero TTL, zero retry/extension intervals and empty key lists are rejected
  before contacting Redis.

### Documentation

- Reworked the README into a guide with an architecture overview, API table,
  guarantees and limitations, error-handling examples and an FAQ.
- Added runnable examples in `examples/basic.rs` and
  `examples/custom_options.rs`.
- CI now builds the documentation with warnings denied.

### Changed

- Upgraded dependencies to their latest versions: tokio 1.53.3, redis 1.7.1,
  uuid 1.28.0, thiserror 2.0.21. The minimum supported Rust version is now
  1.89.
- Adapted to the redis 1.x `ConnectionManagerConfig::set_max_delay` API, which
  now takes a `std::time::Duration`.
- Trimmed the tokio feature set used by the library.
- Enforced `rustfmt` formatting and `clippy -D warnings` in CI.
- README documents the single-instance, failover, eviction and fencing
  limitations.
