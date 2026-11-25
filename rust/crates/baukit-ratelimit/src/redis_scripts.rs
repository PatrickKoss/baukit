//! Lua scripts the Redis store runs atomically.

/// Token-bucket body; mirrors [`crate::kernels::bucket::admit`] in scaled integer units.
///
/// Expects `now_ms` in scope. ARGV: scaled capacity, rate per ms, cost, TTL in ms.
macro_rules! token_bucket_body {
    () => {
        r#"
local capacity = tonumber(ARGV[1])
local rate = tonumber(ARGV[2])
local cost = tonumber(ARGV[3])
local ttl_ms = tonumber(ARGV[4])
local values = redis.call('HMGET', KEYS[1], 'scaled_tokens', 'last_ms')
local tokens = tonumber(values[1]) or capacity
local last_ms = tonumber(values[2]) or now_ms
local elapsed_ms = math.max(0, now_ms - last_ms)
tokens = math.min(capacity, tokens + (elapsed_ms * rate))

local allowed = 0
local retry_ms = 0
if tokens >= cost then
  allowed = 1
  tokens = tokens - cost
else
  retry_ms = math.ceil((cost - tokens) / rate)
end

redis.call('HSET', KEYS[1], 'scaled_tokens', string.format('%d', tokens),
  'last_ms', string.format('%d', math.max(now_ms, last_ms)))
redis.call('HDEL', KEYS[1], 'tokens', 'last_refill_ms')
redis.call('PEXPIRE', KEYS[1], ttl_ms)
return {allowed, math.floor(tokens / cost), retry_ms}
"#
    };
}

pub(crate) const TOKEN_BUCKET_SCRIPT: &str = concat!(
    "local current = redis.call('TIME')\n",
    "local now_ms = (current[1] * 1000) + math.floor(current[2] / 1000)\n",
    token_bucket_body!()
);

/// The token-bucket script with `now_ms` taken from ARGV[5], for parity tests.
#[cfg(test)]
pub(crate) const TOKEN_BUCKET_SCRIPT_AT: &str =
    concat!("local now_ms = tonumber(ARGV[5])\n", token_bucket_body!());

pub(crate) const AMOUNT_BUDGET_SCRIPT: &str = r#"
local current = tonumber(redis.call('GET', KEYS[1])) or 0
local amount = tonumber(ARGV[1])
local limit = tonumber(ARGV[2])
local allowed = 0

if current <= limit and amount <= (limit - current) then
  current = redis.call('INCRBY', KEYS[1], ARGV[1])
  redis.call('PEXPIREAT', KEYS[1], ARGV[3])
  allowed = 1
end

return {allowed, limit - math.min(current, limit)}
"#;

pub(crate) const RELEASE_AMOUNT_SCRIPT: &str = r#"
local current = tonumber(redis.call('GET', KEYS[1])) or 0
local amount = tonumber(ARGV[1])
local limit = tonumber(ARGV[2])

if current > 0 then
  current = math.max(0, current - amount)
  redis.call('SET', KEYS[1], current)
  redis.call('PEXPIREAT', KEYS[1], ARGV[3])
end

return {1, limit - math.min(current, limit)}
"#;
