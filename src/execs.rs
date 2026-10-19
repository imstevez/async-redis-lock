use std::time::Duration;

use redis::ConnectionLike as SyncConnectionLike;
use redis::Script;
use redis::aio::ConnectionLike as AsyncConnectionLike;
use tokio::time::{Instant, sleep};

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

pub(crate) async fn lock<C: AsyncConnectionLike>(
    conn: &mut C,
    keys: &[Vec<u8>],
    ttl: Duration,
    retry_interval: Duration,
    retry_timeout: Option<Duration>,
) -> Result<String> {
    let ttl_ms = ttl_millis(ttl)?;
    let lock_id = uuid::Uuid::new_v4().to_string();

    let script = Script::new(LOCK);
    let mut invoke = script.prepare_invoke();
    for key in keys {
        invoke.key(key);
    }
    invoke.arg(&lock_id);
    invoke.arg(ttl_ms);

    let deadline = retry_timeout.map(|timeout| Instant::now() + timeout);

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
) -> Result<()> {
    let ttl_ms = ttl_millis(ttl)?;

    let script = Script::new(EXTEND);
    let mut invoke = script.prepare_invoke();
    for key in keys {
        invoke.key(key);
    }
    invoke.arg(lock_id);
    invoke.arg(ttl_ms);

    let extended: i32 = invoke.invoke_async(conn).await?;
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
) -> Result<()> {
    let script = Script::new(UNLOCK);
    let mut invoke = script.prepare_invoke();
    for key in keys {
        invoke.key(key);
    }
    invoke.arg(lock_id);

    let deleted: i64 = invoke.invoke_async(conn).await?;
    check_unlocked(deleted, keys.len())
}

pub(crate) fn unlock_sync<C: SyncConnectionLike>(
    conn: &mut C,
    keys: &[Vec<u8>],
    lock_id: &str,
) -> Result<()> {
    let script = Script::new(UNLOCK);
    let mut invoke = script.prepare_invoke();
    for key in keys {
        invoke.key(key);
    }
    invoke.arg(lock_id);

    let deleted: i64 = invoke.invoke(conn)?;
    check_unlocked(deleted, keys.len())
}
