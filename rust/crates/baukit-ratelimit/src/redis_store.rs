use std::{
    fmt,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use redis::{
    aio::{ConnectionManager, MultiplexedConnection},
    sentinel::{SentinelClient, SentinelServerType},
};
use tokio::sync::{Mutex, RwLock};

use crate::{
    AmountBudgetStore, AmountBudgetStoreDecision, Quota, RateLimitDecision, RateLimitStore,
    RateLimitStoreError,
};

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

const AMOUNT_BUDGET_SCRIPT: &str = r#"
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

/// Redis adapter using one atomic Lua token-bucket script.
///
/// Direct `redis://` connections use Redis' reconnecting connection manager.
/// `redis+sentinel://` connections discover the named master through Sentinel
/// and re-resolve it once after a failed decision.
#[derive(Clone)]
pub struct RedisRateLimitStore {
    connection: StoreConnection,
}

#[derive(Clone)]
enum StoreConnection {
    Direct(ConnectionManager),
    Sentinel(Arc<SentinelState>),
}

struct SentinelState {
    client: Mutex<SentinelClient>,
    current: RwLock<ResolvedMaster>,
}

struct ResolvedMaster {
    connection: MultiplexedConnection,
    generation: u64,
}

#[derive(Debug, Eq, PartialEq)]
enum RedisTarget<'a> {
    Direct(&'a str),
    Sentinel(SentinelTarget),
}

#[derive(Debug, Eq, PartialEq)]
struct SentinelTarget {
    sentinels: Vec<String>,
    master_name: String,
}

impl RedisRateLimitStore {
    /// Connects to the Redis target selected by `redis_url`.
    ///
    /// `redis://` retains the direct reconnecting connection-manager mode.
    /// `redis+sentinel://host1:26379,host2:26379/mymaster` discovers the
    /// writable master through the listed Sentinel nodes. Sentinel URLs do not
    /// support authentication, database selection, query strings, or fragments.
    pub async fn connect(redis_url: &str) -> Result<Self, RateLimitStoreError> {
        match parse_redis_target(redis_url)? {
            RedisTarget::Direct(redis_url) => {
                let client = redis::Client::open(redis_url).map_err(RateLimitStoreError::new)?;
                let connection = client
                    .get_connection_manager()
                    .await
                    .map_err(RateLimitStoreError::new)?;
                Ok(Self::from_connection_manager(connection))
            }
            RedisTarget::Sentinel(target) => {
                Self::connect_sentinel(target.sentinels, &target.master_name).await
            }
        }
    }

    /// Connects through Sentinel and discovers the writable Redis master.
    ///
    /// Each Sentinel endpoint must be an unauthenticated `host:port` string;
    /// bracketed IPv6 addresses such as `[::1]:26379` are accepted. At least
    /// one endpoint and a non-empty master name are required.
    pub async fn connect_sentinel<I, S>(
        sentinels: I,
        master_name: &str,
    ) -> Result<Self, RateLimitStoreError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        validate_master_name(master_name)?;
        let sentinels = sentinels
            .into_iter()
            .map(|sentinel| normalize_sentinel_endpoint(sentinel.as_ref()))
            .collect::<Result<Vec<_>, _>>()?;
        if sentinels.is_empty() {
            return Err(configuration_error(
                "Redis Sentinel configuration requires at least one sentinel host",
            ));
        }

        let mut client = SentinelClient::build(
            sentinels,
            master_name.to_owned(),
            None,
            SentinelServerType::Master,
        )
        .map_err(RateLimitStoreError::new)?;
        let connection = client
            .get_async_connection()
            .await
            .map_err(RateLimitStoreError::new)?;
        Ok(Self {
            connection: StoreConnection::Sentinel(Arc::new(SentinelState {
                client: Mutex::new(client),
                current: RwLock::new(ResolvedMaster {
                    connection,
                    generation: 0,
                }),
            })),
        })
    }

    /// Creates an adapter from an existing Redis connection manager.
    #[must_use]
    pub const fn from_connection_manager(connection: ConnectionManager) -> Self {
        Self {
            connection: StoreConnection::Direct(connection),
        }
    }
}

impl fmt::Debug for RedisRateLimitStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mode = match &self.connection {
            StoreConnection::Direct(_) => "direct",
            StoreConnection::Sentinel(_) => "sentinel",
        };
        formatter
            .debug_struct("RedisRateLimitStore")
            .field("mode", &mode)
            .finish_non_exhaustive()
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
            let (allowed, remaining, retry_ms) = match &self.connection {
                StoreConnection::Direct(connection) => {
                    let mut connection = connection.clone();
                    eval_token_bucket(&mut connection, key, quota, period_ms, ttl_ms).await
                }
                StoreConnection::Sentinel(state) => {
                    eval_with_sentinel_retry(state, key, quota, period_ms, ttl_ms).await
                }
            }
            .map_err(RateLimitStoreError::new)?;
            Ok(if allowed == 1 {
                RateLimitDecision::allowed(remaining)
            } else {
                RateLimitDecision::limited(remaining, Duration::from_millis(retry_ms))
            })
        })
    }
}

impl AmountBudgetStore for RedisRateLimitStore {
    fn check_and_consume_amount<'a>(
        &'a self,
        key: &'a str,
        amount: u64,
        limit: u64,
        now: SystemTime,
        reset_at: SystemTime,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<AmountBudgetStoreDecision, RateLimitStoreError>>
                + Send
                + 'a,
        >,
    > {
        Box::pin(async move {
            if reset_at <= now {
                return Err(RateLimitStoreError::unavailable(
                    "amount-budget reset must be after the current time",
                ));
            }
            let amount = i64::try_from(amount).map_err(|_| {
                RateLimitStoreError::unavailable("amount-budget amount exceeds Redis integer range")
            })?;
            let limit = i64::try_from(limit).map_err(|_| {
                RateLimitStoreError::unavailable("amount-budget limit exceeds Redis integer range")
            })?;
            let reset_at_ms = system_time_millis_ceil(reset_at)?;
            let (allowed, remaining) = match &self.connection {
                StoreConnection::Direct(connection) => {
                    let mut connection = connection.clone();
                    eval_amount_budget(&mut connection, key, amount, limit, reset_at_ms).await
                }
                StoreConnection::Sentinel(state) => {
                    eval_amount_with_sentinel_retry(state, key, amount, limit, reset_at_ms).await
                }
            }
            .map_err(RateLimitStoreError::new)?;
            Ok(AmountBudgetStoreDecision {
                allowed: allowed == 1,
                remaining,
            })
        })
    }
}

async fn eval_with_sentinel_retry(
    state: &SentinelState,
    key: &str,
    quota: Quota,
    period_ms: u64,
    ttl_ms: u64,
) -> redis::RedisResult<(i64, u64, u64)> {
    let (mut connection, generation) = {
        let current = state.current.read().await;
        (current.connection.clone(), current.generation)
    };
    match eval_token_bucket(&mut connection, key, quota, period_ms, ttl_ms).await {
        Ok(decision) => Ok(decision),
        Err(_) => {
            let mut client = state.client.lock().await;
            let mut current = state.current.write().await;
            if current.generation == generation {
                current.connection = client.get_async_connection().await?;
                current.generation = current.generation.wrapping_add(1);
            }
            let mut connection = current.connection.clone();
            drop(current);
            drop(client);
            eval_token_bucket(&mut connection, key, quota, period_ms, ttl_ms).await
        }
    }
}

async fn eval_amount_with_sentinel_retry(
    state: &SentinelState,
    key: &str,
    amount: i64,
    limit: i64,
    reset_at_ms: i64,
) -> redis::RedisResult<(i64, u64)> {
    let (mut connection, generation) = {
        let current = state.current.read().await;
        (current.connection.clone(), current.generation)
    };
    match eval_amount_budget(&mut connection, key, amount, limit, reset_at_ms).await {
        Ok(decision) => Ok(decision),
        Err(_) => {
            let mut client = state.client.lock().await;
            let mut current = state.current.write().await;
            if current.generation == generation {
                current.connection = client.get_async_connection().await?;
                current.generation = current.generation.wrapping_add(1);
            }
            let mut connection = current.connection.clone();
            drop(current);
            drop(client);
            eval_amount_budget(&mut connection, key, amount, limit, reset_at_ms).await
        }
    }
}

async fn eval_token_bucket<C>(
    connection: &mut C,
    key: &str,
    quota: Quota,
    period_ms: u64,
    ttl_ms: u64,
) -> redis::RedisResult<(i64, u64, u64)>
where
    C: redis::aio::ConnectionLike + Send + Unpin,
{
    redis::cmd("EVAL")
        .arg(TOKEN_BUCKET_SCRIPT)
        .arg(1)
        .arg(key)
        .arg(quota.capacity())
        .arg(quota.requests_per_period())
        .arg(period_ms)
        .arg(ttl_ms)
        .query_async(connection)
        .await
}

async fn eval_amount_budget<C>(
    connection: &mut C,
    key: &str,
    amount: i64,
    limit: i64,
    reset_at_ms: i64,
) -> redis::RedisResult<(i64, u64)>
where
    C: redis::aio::ConnectionLike + Send + Unpin,
{
    redis::cmd("EVAL")
        .arg(AMOUNT_BUDGET_SCRIPT)
        .arg(1)
        .arg(key)
        .arg(amount)
        .arg(limit)
        .arg(reset_at_ms)
        .query_async(connection)
        .await
}

fn parse_redis_target(redis_url: &str) -> Result<RedisTarget<'_>, RateLimitStoreError> {
    let Some((scheme, remainder)) = redis_url.split_once("://") else {
        return Err(configuration_error(
            "Redis URL must use the redis:// or redis+sentinel:// scheme",
        ));
    };
    match scheme {
        "redis" => Ok(RedisTarget::Direct(redis_url)),
        "redis+sentinel" => parse_sentinel_url(remainder).map(RedisTarget::Sentinel),
        other => Err(configuration_error(format!(
            "unsupported Redis URL scheme `{other}`; expected redis:// or redis+sentinel://"
        ))),
    }
}

fn parse_sentinel_url(remainder: &str) -> Result<SentinelTarget, RateLimitStoreError> {
    if remainder.contains(['?', '#']) {
        return Err(configuration_error(
            "Redis Sentinel URLs do not support query strings or fragments",
        ));
    }
    let Some((authority, master_name)) = remainder.split_once('/') else {
        return Err(configuration_error(
            "Redis Sentinel URL must end with a non-empty /<master-name>",
        ));
    };
    if authority.contains('@') {
        return Err(configuration_error(
            "Redis Sentinel URLs do not support authentication",
        ));
    }
    if master_name.contains('/') {
        return Err(configuration_error(
            "Redis Sentinel URLs support only one master-name path segment; database paths are unsupported",
        ));
    }
    validate_master_name(master_name)?;
    let sentinels = authority
        .split(',')
        .map(|endpoint| {
            normalize_sentinel_endpoint(endpoint)?;
            Ok(endpoint.to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if sentinels.is_empty() {
        return Err(configuration_error(
            "Redis Sentinel URL requires at least one sentinel host",
        ));
    }
    Ok(SentinelTarget {
        sentinels,
        master_name: master_name.to_owned(),
    })
}

fn normalize_sentinel_endpoint(endpoint: &str) -> Result<String, RateLimitStoreError> {
    if endpoint.is_empty() {
        return Err(configuration_error(
            "Redis Sentinel configuration contains an empty sentinel host",
        ));
    }
    if endpoint.trim() != endpoint
        || endpoint.contains(['/', '@', '?', '#', ','])
        || endpoint.chars().any(char::is_whitespace)
    {
        return Err(configuration_error(format!(
            "invalid Redis Sentinel endpoint `{endpoint}`; expected unauthenticated host:port"
        )));
    }

    let (host, port) = if let Some(bracketed) = endpoint.strip_prefix('[') {
        let Some((host, port)) = bracketed.split_once("]:") else {
            return Err(configuration_error(format!(
                "invalid Redis Sentinel endpoint `{endpoint}`; expected [IPv6-address]:port"
            )));
        };
        if host.parse::<std::net::Ipv6Addr>().is_err() {
            return Err(configuration_error(format!(
                "invalid Redis Sentinel IPv6 address in endpoint `{endpoint}`"
            )));
        }
        (host, port)
    } else {
        let Some((host, port)) = endpoint.rsplit_once(':') else {
            return Err(configuration_error(format!(
                "Redis Sentinel endpoint `{endpoint}` is missing an explicit port"
            )));
        };
        if host.is_empty() || host.contains(':') {
            return Err(configuration_error(format!(
                "invalid Redis Sentinel endpoint `{endpoint}`; IPv6 addresses must be bracketed"
            )));
        }
        (host, port)
    };
    if host.is_empty() {
        return Err(configuration_error(format!(
            "Redis Sentinel endpoint `{endpoint}` has an empty host"
        )));
    }
    let port = port.parse::<u16>().map_err(|_| {
        configuration_error(format!(
            "Redis Sentinel endpoint `{endpoint}` has an invalid port"
        ))
    })?;
    if port == 0 {
        return Err(configuration_error(format!(
            "Redis Sentinel endpoint `{endpoint}` has port zero"
        )));
    }
    Ok(format!("redis://{endpoint}/"))
}

fn validate_master_name(master_name: &str) -> Result<(), RateLimitStoreError> {
    if master_name.is_empty() {
        return Err(configuration_error(
            "Redis Sentinel master name must not be empty",
        ));
    }
    if master_name.trim() != master_name
        || master_name.contains(['/', '?', '#', '%'])
        || master_name.chars().any(char::is_whitespace)
    {
        return Err(configuration_error(
            "Redis Sentinel master name must be one unescaped, non-whitespace path segment",
        ));
    }
    Ok(())
}

fn configuration_error(message: impl Into<String>) -> RateLimitStoreError {
    RateLimitStoreError::new(std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        message.into(),
    ))
}

fn duration_millis_ceil(duration: Duration) -> u64 {
    let rounded = duration.as_nanos().div_ceil(1_000_000);
    u64::try_from(rounded.clamp(1, i64::MAX as u128)).unwrap_or(i64::MAX as u64)
}

fn system_time_millis_ceil(time: SystemTime) -> Result<i64, RateLimitStoreError> {
    let duration = time.duration_since(UNIX_EPOCH).map_err(|_| {
        RateLimitStoreError::unavailable("amount-budget reset precedes the Unix epoch")
    })?;
    let millis = duration.as_nanos().div_ceil(1_000_000);
    i64::try_from(millis).map_err(|_| {
        RateLimitStoreError::unavailable("amount-budget reset exceeds Redis timestamp range")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multi_host_sentinel_url() -> Result<(), RateLimitStoreError> {
        assert_eq!(
            parse_redis_target(
                "redis+sentinel://sentinel-1:26379,sentinel-2:26380,sentinel-3:26381/mymaster"
            )?,
            RedisTarget::Sentinel(SentinelTarget {
                sentinels: vec![
                    "sentinel-1:26379".to_owned(),
                    "sentinel-2:26380".to_owned(),
                    "sentinel-3:26381".to_owned(),
                ],
                master_name: "mymaster".to_owned(),
            })
        );
        Ok(())
    }

    #[test]
    fn parses_single_host_sentinel_url() -> Result<(), RateLimitStoreError> {
        assert_eq!(
            parse_redis_target("redis+sentinel://127.0.0.1:26379/mymaster")?,
            RedisTarget::Sentinel(SentinelTarget {
                sentinels: vec!["127.0.0.1:26379".to_owned()],
                master_name: "mymaster".to_owned(),
            })
        );
        Ok(())
    }

    #[test]
    fn rejects_missing_sentinel_master_name() {
        let error = parse_redis_target("redis+sentinel://127.0.0.1:26379/")
            .expect_err("missing master name must fail");
        assert!(error.to_string().contains("master name must not be empty"));
    }

    #[test]
    fn rejects_unknown_scheme() {
        let error = parse_redis_target("rediss+sentinel://127.0.0.1:26379/mymaster")
            .expect_err("unsupported TLS Sentinel scheme must fail");
        assert!(error.to_string().contains("unsupported Redis URL scheme"));
    }

    #[test]
    fn routes_plain_redis_url_to_direct_mode() -> Result<(), RateLimitStoreError> {
        let url = "redis://user:password@127.0.0.1:6379/3";
        assert_eq!(parse_redis_target(url)?, RedisTarget::Direct(url));
        Ok(())
    }

    #[test]
    fn rejects_unsupported_sentinel_url_parts() {
        for url in [
            "redis+sentinel://user@127.0.0.1:26379/mymaster",
            "redis+sentinel://127.0.0.1:26379/mymaster/2",
            "redis+sentinel://127.0.0.1:26379/mymaster?password=secret",
            "redis+sentinel:///mymaster",
        ] {
            assert!(parse_redis_target(url).is_err(), "URL should fail: {url}");
        }
    }
}
