use std::future::Future;
use std::sync::LazyLock;
use std::time::Duration;

use redis::ConnectionLike as SyncConnectionLike;
use redis::Script;
use redis::aio::ConnectionLike as AsyncConnectionLike;
use tokio::time::{Instant, sleep, timeout};

use crate::error::{Error, Result};

const OK_RET: i32 = 1;

const LOCK: &str = r#"
for i = 1, #KEYS do
    if redis.call("EXISTS", KEYS[i]) > 0  then
        return 0
    end
end
for i = 1, #KEYS do
    redis.call("SET", KEYS[i], ARGV[1], "PX", ARGV[2])
end
return 1
"#;

const EXTEND: &str = r#"
for i = 1, #KEYS do
    if redis.call("GET", KEYS[i]) ~= ARGV[1] then
        return 0
    end
end
for i = 1, #KEYS do
    redis.call("PEXPIRE", KEYS[i], ARGV[2])
end
return 1
"#;

// Deletes every key still owned by this lock, even if some keys have already
// expired or were taken over by another owner. Returns the number of deleted
// keys so the caller can tell whether the whole lock was still intact.
const UNLOCK: &str = r#"
local deleted = 0
for i = 1, #KEYS do
    if redis.call("GET", KEYS[i]) == ARGV[1] then
        redis.call("DEL", KEYS[i])
        deleted = deleted + 1
    end
end
return deleted
"#;

// `Script::new` computes the SHA1 of the Lua source. Build each script once
// and share it: `Script` is immutable and `prepare_invoke` only borrows it.
static LOCK_SCRIPT: LazyLock<Script> = LazyLock::new(|| Script::new(LOCK));
static EXTEND_SCRIPT: LazyLock<Script> = LazyLock::new(|| Script::new(EXTEND));
static UNLOCK_SCRIPT: LazyLock<Script> = LazyLock::new(|| Script::new(UNLOCK));

fn ttl_millis(ttl: Duration) -> Result<u64> {
    let millis = u64::try_from(ttl.as_millis()).unwrap_or(u64::MAX);
    if millis == 0 {
        return Err(Error::InvalidOptions("ttl must be at least 1ms"));
    }
    Ok(millis)
}

fn check_unlocked(deleted: i64, expected: usize) -> Result<()> {
    if deleted == expected as i64 {
        Ok(())
    } else {
        Err(Error::IdNotFound)
    }
}

/// Runs a Redis command with a hard upper bound on how long it may take.
///
/// `ConnectionManager` may wait for several reconnect attempts before a command
/// fails, which can be much longer than the lock TTL. Timing out here keeps
/// extension and release bounded; Redis may still apply the command, but the
/// owner-token check makes a retry safe.
async fn with_timeout<F, T>(duration: Duration, future: F) -> Result<T>
where
    F: Future<Output = redis::RedisResult<T>>,
{
    match timeout(duration, future).await {
        Ok(result) => result.map_err(Error::from),
        Err(_) => Err(Error::OperationTimeout),
    }
}

pub(crate) async fn lock<C: AsyncConnectionLike>(
    conn: &mut C,
    keys: &[Vec<u8>],
    ttl: Duration,
    retry_interval: Duration,
    retry_timeout: Option<Duration>,
) -> Result<String> {
    let ttl_ms = ttl_millis(ttl)?;
    let lock_id = uuid::Uuid::new_v4().to_string();

    let mut invoke = LOCK_SCRIPT.prepare_invoke();
    for key in keys {
        invoke.key(key);
    }
    invoke.arg(&lock_id);
    invoke.arg(ttl_ms);

    // An unrepresentable timeout (for example `Duration::MAX`) is treated as
    // "no timeout" instead of panicking while adding it to `Instant::now()`.
    let deadline = retry_timeout.and_then(|timeout| Instant::now().checked_add(timeout));

    loop {
        let acquired: i32 = invoke.invoke_async(conn).await?;
        if acquired == OK_RET {
            return Ok(lock_id);
        }

        match deadline {
            Some(deadline) => {
                let now = Instant::now();
                if now >= deadline {
                    return Err(Error::Timeout);
                }
                // Never sleep past the deadline, so the configured timeout is
                // honored even when `retry_interval` is larger than it.
                sleep((deadline - now).min(retry_interval)).await;
            }
            None => sleep(retry_interval).await,
        }
    }
}

pub(crate) async fn extend<C: AsyncConnectionLike>(
    conn: &mut C,
    keys: &[Vec<u8>],
    lock_id: &str,
    ttl: Duration,
    operation_timeout: Duration,
) -> Result<()> {
    let ttl_ms = ttl_millis(ttl)?;

    let mut invoke = EXTEND_SCRIPT.prepare_invoke();
    for key in keys {
        invoke.key(key);
    }
    invoke.arg(lock_id);
    invoke.arg(ttl_ms);

    let extended: i32 = with_timeout(operation_timeout, invoke.invoke_async(conn)).await?;
    if extended == OK_RET {
        Ok(())
    } else {
        Err(Error::IdNotFound)
    }
}

pub(crate) async fn unlock<C: AsyncConnectionLike>(
    conn: &mut C,
    keys: &[Vec<u8>],
    lock_id: &str,
    operation_timeout: Duration,
) -> Result<()> {
    let mut invoke = UNLOCK_SCRIPT.prepare_invoke();
    for key in keys {
        invoke.key(key);
    }
    invoke.arg(lock_id);

    let deleted: i64 = with_timeout(operation_timeout, invoke.invoke_async(conn)).await?;
    check_unlocked(deleted, keys.len())
}

pub(crate) fn unlock_sync<C: SyncConnectionLike>(
    conn: &mut C,
    keys: &[Vec<u8>],
    lock_id: &str,
) -> Result<()> {
    let mut invoke = UNLOCK_SCRIPT.prepare_invoke();
    for key in keys {
        invoke.key(key);
    }
    invoke.arg(lock_id);

    let deleted: i64 = invoke.invoke(conn)?;
    check_unlocked(deleted, keys.len())
}

#[cfg(test)]
mod tests {
    use std::future::pending;

    use super::*;

    #[tokio::test]
    async fn operation_timeout_maps_to_error() {
        let result = with_timeout(
            Duration::from_millis(1),
            pending::<redis::RedisResult<()>>(),
        )
        .await;
        assert_eq!(result, Err(Error::OperationTimeout));
    }
}
