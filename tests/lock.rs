//! Integration tests for the public API.
//!
//! They require a running Redis server. Set `REDIS_URL` to point at it; the
//! default is `redis://127.0.0.1:6379/0`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_redis_lock::{Error, Locker, Options};
use tokio::time::sleep;

fn redis_url() -> String {
    std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379/0".to_string())
}

async fn locker() -> Locker {
    Locker::from_redis_url(&redis_url())
        .await
        .expect("failed to connect to Redis; start redis-server or set REDIS_URL")
}

/// All tests use unique keys so they can run in parallel and against a shared
/// server.
fn key(name: &str) -> String {
    format!("async-redis-lock:test:{}:{}", name, uuid::Uuid::new_v4())
}

#[tokio::test]
async fn lock_is_exclusive() {
    let locker = locker().await;
    let key = key("exclusive");

    let lock = locker.acquire(key.as_str()).await.expect("acquire lock");

    let started = Instant::now();
    let err = locker
        .acquire(key.as_str())
        .await
        .expect_err("second acquisition must fail");
    assert_eq!(err, Error::Timeout);
    assert!(
        started.elapsed() >= Duration::from_millis(900),
        "acquisition should respect the default 1s timeout, took {:?}",
        started.elapsed()
    );

    lock.release().await.expect("release lock");

    let lock = locker
        .acquire(key.as_str())
        .await
        .expect("acquire lock after release");
    lock.release().await.expect("release lock");
}

#[tokio::test]
async fn drop_releases_lock() {
    let locker = locker().await;
    let key = key("drop");

    {
        let _lock = locker.acquire(key.as_str()).await.expect("acquire lock");
    }

    // `Drop` schedules the release on the current runtime.
    sleep(Duration::from_millis(100)).await;

    let lock = locker
        .acquire(key.as_str())
        .await
        .expect("acquire lock after drop");
    lock.release().await.expect("release lock");
}

#[test]
fn drop_outside_runtime_uses_blocking_fallback() {
    let url = redis_url();
    let key = key("drop-outside-runtime");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let lock = runtime.block_on(async {
        let locker = Locker::from_redis_url(&url).await.expect("locker");
        locker.acquire(key.as_str()).await.expect("acquire lock")
    });

    // The lock is dropped after its runtime is gone, so `Drop` has to fall back
    // to a blocking connection.
    drop(runtime);
    drop(lock);

    let mut conn = redis::Client::open(url)
        .expect("redis client")
        .get_connection()
        .expect("sync connection");
    let exists: i64 = redis::cmd("EXISTS")
        .arg(key.as_str())
        .query(&mut conn)
        .expect("exists");
    assert_eq!(exists, 0, "the blocking fallback must release the lock");
}

#[test]
fn public_types_are_thread_safe() {
    fn assert_send_sync<T: Send + Sync>() {}
    fn assert_send<T: Send>() {}

    // `Locker` is meant to be shared between tasks.
    assert_send_sync::<Locker>();
    // `Lock` can be moved between tasks and dropped from any thread.
    assert_send::<async_redis_lock::Lock>();
}

#[tokio::test]
async fn lock_expires_and_is_reported_lost() {
    let locker = locker().await;
    let key = key("lost");

    let opts = Options::new()
        .ttl(Duration::from_millis(500))
        .extend(Duration::from_secs(1));
    let lock = locker
        .acquire_with_options(&opts, key.as_str())
        .await
        .expect("acquire lock");
    assert!(!lock.is_lost());

    // The lock expires at 500ms and the extension attempt at 1s discovers it.
    sleep(Duration::from_millis(1500)).await;
    assert!(lock.is_lost(), "extension should report the lost lock");

    let new_lock = locker
        .acquire(key.as_str())
        .await
        .expect("acquire expired lock");
    new_lock.release().await.expect("release lock");
}

#[tokio::test]
async fn release_after_expiry_reports_id_not_found() {
    let locker = locker().await;
    let key = key("expired-release");

    let opts = Options::new()
        .ttl(Duration::from_millis(200))
        .extend(Duration::from_secs(5));
    let lock = locker
        .acquire_with_options(&opts, key.as_str())
        .await
        .expect("acquire lock");

    sleep(Duration::from_millis(400)).await;

    let err = lock.release().await.expect_err("release must fail");
    assert_eq!(err, Error::IdNotFound);
}

#[tokio::test]
async fn release_cleans_up_owned_keys_after_partial_loss() {
    let locker = locker().await;
    let key_a = key("partial-a");
    let key_b = key("partial-b");
    let keys = [key_a.as_str(), key_b.as_str()];

    let opts = Options::new()
        .ttl(Duration::from_secs(30))
        .extend(Duration::from_secs(10));
    let lock = locker
        .acquire_with_options(&opts, &keys[..])
        .await
        .expect("acquire multi-key lock");

    // Simulate key A expiring and being taken over by another owner.
    let mut conn = redis::Client::open(redis_url())
        .expect("redis client")
        .get_connection()
        .expect("sync connection");
    redis::cmd("SET")
        .arg(key_a.as_str())
        .arg("other-owner")
        .arg("PX")
        .arg(30_000)
        .query::<()>(&mut conn)
        .expect("overwrite key A");

    let err = lock.release().await.expect_err("partial release must fail");
    assert_eq!(err, Error::IdNotFound);

    // Key B is still owned by the lock, so it must not be left behind.
    let exists: i64 = redis::cmd("EXISTS")
        .arg(key_b.as_str())
        .query(&mut conn)
        .expect("exists");
    assert_eq!(exists, 0, "keys still owned by the lock must be deleted");

    let _: i64 = redis::cmd("DEL")
        .arg(&keys[..])
        .query(&mut conn)
        .expect("cleanup");
}

#[tokio::test]
async fn invalid_options_are_rejected() {
    let locker = locker().await;
    let key = key("invalid-options");

    let opts = Options::new().ttl(Duration::ZERO);
    assert_eq!(
        locker
            .acquire_with_options(&opts, key.as_str())
            .await
            .expect_err("zero ttl"),
        Error::InvalidOptions("ttl must be at least 1ms")
    );

    let opts = Options::new().retry(Duration::ZERO);
    assert!(matches!(
        locker
            .acquire_with_options(&opts, key.as_str())
            .await
            .expect_err("zero retry"),
        Error::InvalidOptions(_)
    ));

    let opts = Options::new().extend(Duration::ZERO);
    assert!(matches!(
        locker
            .acquire_with_options(&opts, key.as_str())
            .await
            .expect_err("zero extend"),
        Error::InvalidOptions(_)
    ));

    let opts = Options::new().operation_timeout(Duration::ZERO);
    assert_eq!(
        locker
            .acquire_with_options(&opts, key.as_str())
            .await
            .expect_err("zero operation timeout"),
        Error::InvalidOptions("operation timeout must be at least 1ms")
    );
}

#[tokio::test]
async fn operation_timeout_is_configurable() {
    let locker = locker().await;
    let key = key("operation-timeout");
    let opts = Options::new()
        .ttl(Duration::from_secs(5))
        .extend(Duration::from_secs(1))
        .operation_timeout(Duration::from_millis(200));

    let lock = locker
        .acquire_with_options(&opts, key.as_str())
        .await
        .expect("acquire lock");
    lock.release().await.expect("release lock");
}

#[tokio::test]
async fn huge_acquisition_timeout_does_not_panic() {
    let locker = locker().await;
    let key = key("huge-timeout");
    let opts = Options::new().timeout(Some(Duration::MAX));

    let lock = locker
        .acquire_with_options(&opts, key.as_str())
        .await
        .expect("acquire lock with a huge timeout");
    lock.release().await.expect("release lock");
}

#[tokio::test]
async fn empty_key_list_is_rejected() {
    let locker = locker().await;
    let keys: Vec<String> = Vec::new();

    let err = locker.acquire(keys).await.expect_err("empty keys");
    assert_eq!(err, Error::NoKeys);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_acquire_is_exclusive() {
    let locker = locker().await;
    let key = key("concurrent");
    let in_section = Arc::new(AtomicUsize::new(0));
    let max_in_section = Arc::new(AtomicUsize::new(0));

    let mut handles = Vec::new();
    for _ in 0..4 {
        let locker = locker.clone();
        let key = key.clone();
        let in_section = Arc::clone(&in_section);
        let max_in_section = Arc::clone(&max_in_section);

        handles.push(tokio::spawn(async move {
            let opts = Options::new().timeout(Some(Duration::from_secs(10)));
            let lock = locker
                .acquire_with_options(&opts, key.as_str())
                .await
                .expect("acquire lock");

            let current = in_section.fetch_add(1, Ordering::SeqCst) + 1;
            max_in_section.fetch_max(current, Ordering::SeqCst);

            sleep(Duration::from_millis(25)).await;
            in_section.fetch_sub(1, Ordering::SeqCst);

            lock.release().await.expect("release lock");
        }));
    }

    for handle in handles {
        handle.await.expect("task");
    }

    assert_eq!(
        max_in_section.load(Ordering::SeqCst),
        1,
        "the lock must allow only one holder at a time"
    );
}

#[tokio::test]
async fn duplicate_keys_are_deduplicated() {
    let locker = locker().await;
    let key = key("duplicate-keys");
    let keys = vec![key.as_str(), key.as_str()];

    let lock = locker
        .acquire(keys)
        .await
        .expect("acquire a lock with duplicate keys");
    lock.release().await.expect("release duplicate keys");

    let mut conn = redis::Client::open(redis_url())
        .expect("redis client")
        .get_connection()
        .expect("sync connection");
    let exists: i64 = redis::cmd("EXISTS")
        .arg(key.as_str())
        .query(&mut conn)
        .expect("exists");
    assert_eq!(exists, 0, "the lock key must be deleted");
}

#[tokio::test]
async fn failed_release_is_retried_on_drop() {
    let locker = locker().await;
    let key = key("release-retry");

    // Keep the extension task out of the way for the duration of the test.
    let opts = Options::new()
        .ttl(Duration::from_secs(30))
        .extend(Duration::from_secs(60));
    let lock = locker
        .acquire_with_options(&opts, key.as_str())
        .await
        .expect("acquire lock");

    let mut conn = redis::Client::open(redis_url())
        .expect("redis client")
        .get_connection()
        .expect("sync connection");

    let token: String = redis::cmd("GET")
        .arg(key.as_str())
        .query(&mut conn)
        .expect("read owner token");

    // Turn the key into a list so `UNLOCK`'s GET fails with WRONGTYPE.
    redis::cmd("DEL")
        .arg(key.as_str())
        .query::<()>(&mut conn)
        .expect("delete key");
    redis::cmd("LPUSH")
        .arg(key.as_str())
        .arg("not-a-string")
        .query::<()>(&mut conn)
        .expect("replace key with a list");

    let result = lock.release().await;
    assert!(
        matches!(result, Err(Error::Redis(_))),
        "release must surface the Redis error, got {result:?}"
    );

    // `release` consumed the guard, but its Drop schedules a best-effort
    // retry. Restore the key synchronously before yielding so the retry can
    // succeed, then let the background task run.
    redis::cmd("DEL")
        .arg(key.as_str())
        .query::<()>(&mut conn)
        .expect("delete list");
    redis::cmd("SET")
        .arg(key.as_str())
        .arg(token)
        .arg("PX")
        .arg(30_000)
        .query::<()>(&mut conn)
        .expect("restore owner token");

    // Let the background cleanup task run, polling to keep the test reliable
    // on a loaded CI runner.
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut exists = 1;
    while Instant::now() < deadline {
        exists = redis::cmd("EXISTS")
            .arg(key.as_str())
            .query(&mut conn)
            .expect("exists");
        if exists == 0 {
            break;
        }
        sleep(Duration::from_millis(25)).await;
    }

    if exists != 0 {
        let _: i64 = redis::cmd("DEL")
            .arg(key.as_str())
            .query(&mut conn)
            .expect("cleanup");
    }
    assert_eq!(exists, 0, "Drop must retry the failed release");
}
