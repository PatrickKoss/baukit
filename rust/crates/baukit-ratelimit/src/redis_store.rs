use std::time::Duration;

use redis::aio::ConnectionManager;

use crate::{Quota, RateLimitDecision, RateLimitStore, RateLimitStoreError};

const TOKEN_BUCKET_SCRIPT: &str = r#"
local current = redis.call('TIME')
local now_ms = (current[1] * 1000) + math.floor(current[2] / 1000)
local capacity = tonumber(ARGV[1])
local requests = tonumber(ARGV[2])
local period_ms = tonumber(ARGV[3])
local ttl_ms = tonumber(ARGV[4])
local values = redis.call('HMGET', KEYS[1], 'tokens', 'last_refill_ms')
local tokens = tonumber(values[1]) or capacity
local last_refill_ms = tonumber(values[2]) or now_ms
local elapsed_ms = math.max(0, now_ms - last_refill_ms)
tokens = math.min(capacity, tokens + (elapsed_ms * requests / period_ms))

local allowed = 0
local retry_ms = 0
if tokens >= 1 then
  allowed = 1
  tokens = tokens - 1
else
  retry_ms = math.ceil((1 - tokens) * period_ms / requests)
end

redis.call('HSET', KEYS[1], 'tokens', tokens, 'last_refill_ms', now_ms)
redis.call('PEXPIRE', KEYS[1], ttl_ms)
return {allowed, math.floor(tokens), retry_ms}
"#;

/// Redis connection-manager adapter using one atomic Lua token-bucket script.
#[derive(Clone, Debug)]
pub struct RedisRateLimitStore {
    connection: ConnectionManager,
}

impl RedisRateLimitStore {
    /// Connects a reconnecting Tokio connection manager to `redis_url`.
    pub async fn connect(redis_url: &str) -> Result<Self, RateLimitStoreError> {
        let client = redis::Client::open(redis_url).map_err(RateLimitStoreError::new)?;
        let connection = client
            .get_connection_manager()
            .await
            .map_err(RateLimitStoreError::new)?;
        Ok(Self { connection })
    }

    /// Creates an adapter from an existing Redis connection manager.
    #[must_use]
    pub const fn from_connection_manager(connection: ConnectionManager) -> Self {
        Self { connection }
    }
}

impl RateLimitStore for RedisRateLimitStore {
    fn check_and_consume<'a>(
        &'a self,
        key: &'a str,
        quota: Quota,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<RateLimitDecision, RateLimitStoreError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            let period_ms = duration_millis_ceil(quota.period());
            let ttl_ms = duration_millis_ceil(quota.idle_ttl());
            let mut connection = self.connection.clone();
            let (allowed, remaining, retry_ms): (i64, u64, u64) = redis::cmd("EVAL")
                .arg(TOKEN_BUCKET_SCRIPT)
                .arg(1)
                .arg(key)
                .arg(quota.capacity())
                .arg(quota.requests_per_period())
                .arg(period_ms)
                .arg(ttl_ms)
                .query_async(&mut connection)
                .await
                .map_err(RateLimitStoreError::new)?;
            Ok(if allowed == 1 {
                RateLimitDecision::allowed(remaining)
            } else {
                RateLimitDecision::limited(remaining, Duration::from_millis(retry_ms))
            })
        })
    }
}

fn duration_millis_ceil(duration: Duration) -> u64 {
    let rounded = duration.as_nanos().div_ceil(1_000_000);
    u64::try_from(rounded.clamp(1, i64::MAX as u128)).unwrap_or(i64::MAX as u64)
}
