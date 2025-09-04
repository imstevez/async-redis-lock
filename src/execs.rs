use crate::error::Error::{IdNotFound, Timeout};
use redis::ConnectionLike as SyncConnectionLike;
use redis::Script;
use redis::aio::ConnectionLike as AsyncConnectionLike;
use std::time::SystemTime;
use tokio::time::{Duration, sleep};

const OK_RET: i32 = 1;

const LOCK: &str = r#"if redis.call("EXISTS", KEYS[1]) > 0  then
    return 0
end
redis.call("SET", KEYS[1], ARGV[1], "PX", ARGV[2])
return 1
"#;

const EXTEND: &str = r#"
if redis.call("GET", KEYS[1]) ~= ARGV[1] then
    return 0
end
return redis.call("PEXPIRE", KEYS[1], ARGV[2])
"#;

const UNLOCK: &str = r#"
if redis.call("GET", KEYS[1]) ~= ARGV[1] then
    return 0
end
return redis.call("DEL", KEYS[1])
"#;

pub async fn lock<T: AsyncConnectionLike>(
    conn: &mut T,
    lock_key: &str,
    duration: Duration,
    retry_interval: Duration,
    retry_timeout: Option<Duration>,
) -> anyhow::Result<String> {
    let lock_id = uuid::Uuid::new_v4().to_string();
    let script = Script::new(LOCK);
    let duration = duration.as_millis() as u64;
    let deadline = retry_timeout.map(|timeout| SystemTime::now() + timeout);

    loop {
        if let OK_RET = script
            .key(lock_key)
            .arg(&lock_id)
            .arg(duration)
            .invoke_async(conn)
            .await?
        {
            break Ok(lock_id);
        }

        if let Some(deadline) = deadline
            && SystemTime::now() >= deadline
        {
            break Err(Timeout.into());
        }

        sleep(retry_interval).await;
    }
}

pub async fn extend<T: AsyncConnectionLike>(
    conn: &mut T,
    lock_key: &str,
    lock_id: &str,
    duration: Duration,
) -> anyhow::Result<()> {
    let script = Script::new(EXTEND);
    let duration = duration.as_millis() as u64;
    match script
        .key(lock_key)
        .arg(lock_id)
        .arg(duration)
        .invoke_async(conn)
        .await?
    {
        OK_RET => Ok(()),
        _ => Err(IdNotFound.into()),
    }
}

pub async fn unlock<T: AsyncConnectionLike>(
    conn: &mut T,
    lock_key: &str,
    lock_id: &str,
) -> anyhow::Result<()> {
    let script = Script::new(EXTEND);
    match script.key(lock_key).arg(lock_id).invoke_async(conn).await? {
        1 => Ok(()),
        _ => Err(IdNotFound.into()),
    }
}

pub fn unlock_sync<T: SyncConnectionLike>(
    conn: &mut T,
    lock_key: &str,
    lock_id: &str,
) -> anyhow::Result<()> {
    let script = Script::new(UNLOCK);
    match script.key(lock_key).arg(lock_id).invoke(conn)? {
        1 => Ok(()),
        _ => Err(IdNotFound.into()),
    }
}
