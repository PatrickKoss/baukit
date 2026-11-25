use std::{
    borrow::Cow,
    fmt,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use redis::{
    RedisConnectionInfo,
    aio::{ConnectionManager, MultiplexedConnection},
    sentinel::{SentinelClient, SentinelNodeConnectionInfo, SentinelServerType},
};
use tokio::sync::{Mutex, RwLock};

use crate::{
    AmountBudgetStore, AmountBudgetStoreDecision, Quota, RateLimitDecision, RateLimitOptions,
    RateLimitStore, RateLimitStoreError,
    kernels::bucket::ScaledQuota,
    redis_scripts::{AMOUNT_BUDGET_SCRIPT, RELEASE_AMOUNT_SCRIPT, TOKEN_BUCKET_SCRIPT},
};

/// Redis adapter using one atomic Lua token-bucket script.
///
/// Direct `redis://` connections use Redis' reconnecting connection manager.
/// `redis+sentinel://` connections discover the named master through Sentinel
/// and re-resolve it once after a failed decision.
///
/// An optional key namespace is prepended to every key of every operation, so
/// a tenant restricted by a Redis ACL pattern `~<namespace>*` never touches
/// keys outside it.
#[derive(Clone)]
pub struct RedisRateLimitStore {
    connection: StoreConnection,
    namespace: Option<Arc<str>>,
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
    credentials: Option<NodeCredentials>,
}

#[derive(Eq, PartialEq)]
struct NodeCredentials {
    username: Option<String>,
    password: String,
}

impl fmt::Debug for NodeCredentials {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeCredentials")
            .field("username", &self.username)
            .field("password", &"[redacted]")
            .finish()
    }
}

impl NodeCredentials {
    fn connection_info(&self) -> SentinelNodeConnectionInfo {
        let mut info = RedisConnectionInfo::default().set_password(&self.password);
        if let Some(username) = &self.username {
            info = info.set_username(username);
        }
        SentinelNodeConnectionInfo::default().set_redis_connection_info(info)
    }
}

impl RedisRateLimitStore {
    /// Connects only when at least one request-limit scope is enabled.
    ///
    /// Fully disabled request limiting returns `None` without parsing the Redis
    /// URL or opening a connection. The store uses
    /// [`RateLimitOptions::redis_key_namespace`] as its key namespace.
    ///
    /// # Errors
    ///
    /// The errors of [`Self::connect`] and [`Self::with_key_namespace`] when
    /// limiting is enabled.
    pub async fn connect_if_enabled(
        options: &RateLimitOptions,
    ) -> Result<Option<Self>, RateLimitStoreError> {
        if !options.is_enabled() {
            return Ok(None);
        }
        Self::connect(options.redis_url())
            .await?
            .with_key_namespace(options.redis_key_namespace())
            .map(Some)
    }

    /// Prepends `namespace` to every key this store reads or writes.
    ///
    /// The empty string removes the namespace. A non-empty namespace must end
    /// with `:` and contain no whitespace or Redis glob characters.
    ///
    /// # Errors
    ///
    /// [`RateLimitStoreError`] when `namespace` is invalid.
    pub fn with_key_namespace(mut self, namespace: &str) -> Result<Self, RateLimitStoreError> {
        baukit_config::validate_redis_key_namespace(namespace).map_err(|reason| {
            configuration_error(format!("invalid Redis key namespace: {reason}"))
        })?;
        self.namespace = (!namespace.is_empty()).then(|| Arc::from(namespace));
        Ok(self)
    }

    fn store_key<'k>(&self, key: &'k str) -> Cow<'k, str> {
        namespaced_key(self.namespace.as_deref(), key)
    }

    /// Connects to the Redis target selected by `redis_url`.
    ///
    /// `redis://` retains the direct reconnecting connection-manager mode.
    /// `redis+sentinel://host1:26379,host2:26379/mymaster` discovers the
    /// writable master through the listed Sentinel nodes.
    /// `redis+sentinel://user:password@host1:26379/mymaster` authenticates to
    /// the discovered data nodes as that ACL user; `:password@` uses the
    /// `default` user. Sentinel nodes themselves are contacted without
    /// credentials. Credentials are taken verbatim (no percent-decoding) and
    /// must not contain `@`, `/`, `,`, `%`, `?`, `#` or whitespace. Sentinel
    /// URLs do not support database selection, query strings, or fragments.
    ///
    /// # Errors
    ///
    /// [`RateLimitStoreError`] when the URL is invalid or unsupported, or the first connection
    /// to Redis or a Sentinel master fails.
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
                let node_info = target
                    .credentials
                    .as_ref()
                    .map(NodeCredentials::connection_info);
                Self::connect_sentinel_nodes(target.sentinels, &target.master_name, node_info).await
            }
        }
    }

    /// Connects through Sentinel and discovers the writable Redis master.
    ///
    /// Each Sentinel endpoint must be an unauthenticated `host:port` string;
    /// bracketed IPv6 addresses such as `[::1]:26379` are accepted. At least
    /// one endpoint and a non-empty master name are required.
    ///
    /// # Errors
    ///
    /// [`RateLimitStoreError`] for an empty master name, no or malformed Sentinel endpoints, or
    /// when master discovery or the connection fails.
    pub async fn connect_sentinel<I, S>(
        sentinels: I,
        master_name: &str,
    ) -> Result<Self, RateLimitStoreError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self::connect_sentinel_nodes(sentinels, master_name, None).await
    }

    async fn connect_sentinel_nodes<I, S>(
        sentinels: I,
        master_name: &str,
        node_info: Option<SentinelNodeConnectionInfo>,
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
            node_info,
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
            namespace: None,
        })
    }

    /// Creates an adapter from an existing Redis connection manager.
    #[must_use]
    pub const fn from_connection_manager(connection: ConnectionManager) -> Self {
        Self {
            connection: StoreConnection::Direct(connection),
            namespace: None,
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
            .field("namespace", &self.namespace)
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
            let key = self.store_key(key);
            let key = key.as_ref();
            let scaled = ScaledQuota::from_quota(quota);
            let ttl_ms = duration_millis_ceil(quota.idle_ttl());
            let (allowed, remaining, retry_ms) = match &self.connection {
                StoreConnection::Direct(connection) => {
                    let mut connection = connection.clone();
                    eval_token_bucket(&mut connection, key, scaled, ttl_ms).await
                }
                StoreConnection::Sentinel(state) => {
                    with_sentinel_retry(state, |mut connection| async move {
                        eval_token_bucket(&mut connection, key, scaled, ttl_ms).await
                    })
                    .await
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
            let key = self.store_key(key);
            let key = key.as_ref();
            let (allowed, remaining) = match &self.connection {
                StoreConnection::Direct(connection) => {
                    let mut connection = connection.clone();
                    eval_amount_budget(&mut connection, key, amount, limit, reset_at_ms).await
                }
                StoreConnection::Sentinel(state) => {
                    with_sentinel_retry(state, |mut connection| async move {
                        eval_amount_budget(&mut connection, key, amount, limit, reset_at_ms).await
                    })
                    .await
                }
            }
            .map_err(RateLimitStoreError::new)?;
            Ok(AmountBudgetStoreDecision {
                allowed: allowed == 1,
                remaining,
            })
        })
    }

    fn release_amount<'a>(
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
            let key = self.store_key(key);
            let key = key.as_ref();
            let (allowed, remaining) = match &self.connection {
                StoreConnection::Direct(connection) => {
                    let mut connection = connection.clone();
                    eval_release_amount(&mut connection, key, amount, limit, reset_at_ms).await
                }
                StoreConnection::Sentinel(state) => {
                    with_sentinel_retry(state, |mut connection| async move {
                        eval_release_amount(&mut connection, key, amount, limit, reset_at_ms).await
                    })
                    .await
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

/// Runs `operation` on the current master, re-resolving it after a failure.
///
/// The scripts are not idempotent, so the operation is retried only when the
/// connection was refused and the command therefore never reached Redis. Any
/// other failure re-resolves the master for later calls and returns the error.
async fn with_sentinel_retry<T, F, Fut>(
    state: &SentinelState,
    operation: F,
) -> redis::RedisResult<T>
where
    F: Fn(MultiplexedConnection) -> Fut,
    Fut: std::future::Future<Output = redis::RedisResult<T>>,
{
    let (connection, generation) = {
        let current = state.current.read().await;
        (current.connection.clone(), current.generation)
    };
    let error = match operation(connection).await {
        Ok(value) => return Ok(value),
        Err(error) => error,
    };
    let connection = reresolve_master(state, generation).await?;
    if is_safe_to_retry(&error) {
        operation(connection).await
    } else {
        Err(error)
    }
}

fn is_safe_to_retry(error: &redis::RedisError) -> bool {
    error.is_connection_refusal()
}

async fn reresolve_master(
    state: &SentinelState,
    generation: u64,
) -> redis::RedisResult<MultiplexedConnection> {
    let mut client = state.client.lock().await;
    let mut current = state.current.write().await;
    if current.generation == generation {
        current.connection = client.get_async_connection().await?;
        current.generation = current.generation.wrapping_add(1);
    }
    Ok(current.connection.clone())
}

async fn eval_token_bucket<C>(
    connection: &mut C,
    key: &str,
    scaled: ScaledQuota,
    ttl_ms: u64,
) -> redis::RedisResult<(i64, u64, u64)>
where
    C: redis::aio::ConnectionLike + Send + Unpin,
{
    redis::cmd("EVAL")
        .arg(TOKEN_BUCKET_SCRIPT)
        .arg(1)
        .arg(key)
        .arg(scaled.capacity)
        .arg(scaled.rate)
        .arg(scaled.cost)
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

async fn eval_release_amount<C>(
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
        .arg(RELEASE_AMOUNT_SCRIPT)
        .arg(1)
        .arg(key)
        .arg(amount)
        .arg(limit)
        .arg(reset_at_ms)
        .query_async(connection)
        .await
}

fn namespaced_key<'k>(namespace: Option<&str>, key: &'k str) -> Cow<'k, str> {
    match namespace {
        Some(namespace) => Cow::Owned(format!("{namespace}{key}")),
        None => Cow::Borrowed(key),
    }
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
    let (credentials, authority) = match authority.rsplit_once('@') {
        Some((userinfo, hosts)) => (Some(parse_node_credentials(userinfo)?), hosts),
        None => (None, authority),
    };
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
        credentials,
    })
}

fn parse_node_credentials(userinfo: &str) -> Result<NodeCredentials, RateLimitStoreError> {
    if userinfo.contains(['@', '/', ',', '%', '?', '#'])
        || userinfo.chars().any(char::is_whitespace)
    {
        return Err(configuration_error(
            "Redis Sentinel URL credentials must not contain @ / , % ? # or whitespace",
        ));
    }
    let Some((username, password)) = userinfo.split_once(':') else {
        return Err(configuration_error(
            "Redis Sentinel URL credentials must be user:password@ or :password@",
        ));
    };
    if password.is_empty() {
        return Err(configuration_error(
            "Redis Sentinel URL credentials require a non-empty password",
        ));
    }
    Ok(NodeCredentials {
        username: (!username.is_empty()).then(|| username.to_owned()),
        password: password.to_owned(),
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
    use baukit_config::{RateLimitConfig, Secret};

    use super::*;

    #[tokio::test]
    async fn disabled_scopes_skip_redis_url_parsing_and_connection() {
        let mut config = RateLimitConfig::default();
        config.identity.enabled = false;
        config.ip.enabled = false;
        config.redis_url = Secret::new("not a Redis URL".to_owned());
        let options = RateLimitOptions::from_config(&config).expect("disabled options");

        let store = RedisRateLimitStore::connect_if_enabled(&options)
            .await
            .expect("disabled limiter");

        assert!(store.is_none());
    }

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
                credentials: None,
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
                credentials: None,
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
    fn parses_sentinel_url_with_data_node_credentials() -> Result<(), RateLimitStoreError> {
        let RedisTarget::Sentinel(target) =
            parse_redis_target("redis+sentinel://tenant-a:s3cret@s1:26379,s2:26379/mymaster")?
        else {
            panic!("expected Sentinel target");
        };
        assert_eq!(target.sentinels, ["s1:26379", "s2:26379"]);
        assert_eq!(
            target.credentials,
            Some(NodeCredentials {
                username: Some("tenant-a".to_owned()),
                password: "s3cret".to_owned(),
            })
        );
        assert!(!format!("{target:?}").contains("s3cret"));
        Ok(())
    }

    #[test]
    fn parses_password_only_sentinel_credentials() -> Result<(), RateLimitStoreError> {
        let RedisTarget::Sentinel(target) =
            parse_redis_target("redis+sentinel://:s3cret@s1:26379/mymaster")?
        else {
            panic!("expected Sentinel target");
        };
        assert_eq!(
            target
                .credentials
                .and_then(|credentials| credentials.username),
            None
        );
        Ok(())
    }

    #[test]
    fn namespace_prefixes_store_keys() {
        assert_eq!(
            namespaced_key(Some("tenant-a:"), "rl:id:alice"),
            "tenant-a:rl:id:alice"
        );
        assert_eq!(namespaced_key(None, "rl:id:alice"), "rl:id:alice");
    }

    #[test]
    fn rejects_unsupported_sentinel_url_parts() {
        for url in [
            "redis+sentinel://user@127.0.0.1:26379/mymaster",
            "redis+sentinel://user:@127.0.0.1:26379/mymaster",
            "redis+sentinel://us%40er:pw@127.0.0.1:26379/mymaster",
            "redis+sentinel://127.0.0.1:26379/mymaster/2",
            "redis+sentinel://127.0.0.1:26379/mymaster?password=secret",
            "redis+sentinel:///mymaster",
        ] {
            assert!(parse_redis_target(url).is_err(), "URL should fail: {url}");
        }
    }
}
