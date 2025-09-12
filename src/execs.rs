use crate::error::Error::{IdNotFound, Timeout};
use redis::Script;
use redis::aio::ConnectionLike as AsyncConnectionLike;
use redis::{ConnectionLike as SyncConnectionLike, ToRedisArgs};
use std::time::SystemTime;
use tokio::time::{Duration, sleep};

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

const UNLOCK: &str = r#"
for i = 1, #KEYS do
    if redis.call("GET", KEYS[i]) ~= ARGV[1] then
        return 0
    end
end
for i = 1, #KEYS do
    redis.call("DEL", KEYS[i])
end
return 1
"#;

pub async fn lock<T: AsyncConnectionLike, K: ToRedisArgs>(
    conn: &mut T,
    lock_keys: K,
    duration: Duration,
    retry_interval: Duration,
    retry_timeout: Option<Duration>,
) -> anyhow::Result<String> {
    let lock_id = uuid::Uuid::new_v4().to_string();
    let duration = duration.as_millis() as u64;
    let deadline = retry_timeout.map(|timeout| SystemTime::now() + timeout);
    let script = Script::new(LOCK);
    let mut invoke = script.prepare_invoke();
    lock_keys.to_redis_args().iter().for_each(|key| {
        invoke.key(key);
        ()
    });
    invoke.arg(&lock_id);
    invoke.arg(&duration);
    loop {
        if let OK_RET = invoke.invoke_async(conn).await? {
            break Ok(lock_id);
        }
        if deadline.is_some_and(|deadline| SystemTime::now() > deadline) {
            break Err(Timeout.into());
        }
        sleep(retry_interval).await;
    }
}

pub async fn extend<T: AsyncConnectionLike, K: ToRedisArgs>(
    conn: &mut T,
    lock_keys: K,
    lock_id: &str,
    duration: Duration,
) -> anyhow::Result<()> {
    let duration = duration.as_millis() as u64;
    let script = Script::new(EXTEND);
    let mut invoke = script.prepare_invoke();
    lock_keys.to_redis_args().iter().for_each(|key| {
        invoke.key(key);
        ()
    });
    invoke.arg(lock_id);
    invoke.arg(duration);
    match invoke.invoke_async(conn).await? {
        OK_RET => Ok(()),
        _ => Err(IdNotFound.into()),
    }
}

pub async fn unlock<T: AsyncConnectionLike, K: ToRedisArgs>(
    conn: &mut T,
    lock_keys: K,
    lock_id: &str,
) -> anyhow::Result<()> {
    let script = Script::new(UNLOCK);
    let mut invoke = script.prepare_invoke();
    lock_keys.to_redis_args().iter().for_each(|key| {
        invoke.key(key);
        ()
    });
    invoke.arg(lock_id);
    match invoke.invoke_async(conn).await? {
        1 => Ok(()),
        _ => Err(IdNotFound.into()),
    }
}

pub fn unlock_sync<T: SyncConnectionLike, K: ToRedisArgs>(
    conn: &mut T,
    lock_keys: K,
    lock_id: &str,
) -> anyhow::Result<()> {
    let script = Script::new(UNLOCK);
    let mut invoke = script.prepare_invoke();
    lock_keys.to_redis_args().iter().for_each(|key| {
        invoke.key(key);
        ()
    });
    invoke.arg(lock_id);
    match invoke.invoke(conn)? {
        1 => Ok(()),
        _ => Err(IdNotFound.into()),
    }
}
