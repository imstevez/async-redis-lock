# Changelog

All notable changes to this project are documented in this file.

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
