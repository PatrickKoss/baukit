use std::time::Duration;

use serde::{Deserialize, Deserializer};

const DEFAULT_IPV6_PREFIX_LEN: u8 = 64;
const MAX_IPV6_PREFIX_LEN: u8 = 128;

use crate::{
    Environment, Secret, Validate, ValidationError, ValidationErrors, duration_seconds,
    require_non_zero, validate_redis_key_namespace, validation_result,
};

/// Whether request processing continues when the rate-limit store is unavailable.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitFailMode {
    /// Record the store error and allow the request to continue.
    #[default]
    Open,
    /// Record the store error and reject the request.
    Closed,
}

/// Configuration for one token-bucket rate-limit scope.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct RateLimitScopeConfig {
    /// Whether this scope is evaluated.
    pub enabled: bool,
    /// Tokens refilled during each period.
    pub requests_per_period: u64,
    /// Refill period, in integer seconds on input.
    #[serde(with = "duration_seconds")]
    pub period: Duration,
    /// Extra tokens available above the steady-period request count.
    pub burst: u64,
}

impl RateLimitScopeConfig {
    fn identity_default() -> Self {
        Self {
            enabled: true,
            requests_per_period: 60,
            period: Duration::from_secs(60),
            burst: 10,
        }
    }

    fn ip_default() -> Self {
        Self {
            enabled: true,
            requests_per_period: 6_000,
            period: Duration::from_secs(60),
            burst: 500,
        }
    }

    fn validate_into(&self, path: &str, errors: &mut Vec<ValidationError>) {
        if !self.enabled {
            return;
        }
        require_non_zero(
            self.requests_per_period,
            &format!("{path}.requests_per_period"),
            errors,
        );
        require_non_zero(self.period.as_nanos(), &format!("{path}.period"), errors);
        if self.requests_per_period.checked_add(self.burst).is_none() {
            errors.push(ValidationError::new(
                format!("{path}.burst"),
                "must not overflow the bucket capacity",
            ));
        }
    }
}

impl Default for RateLimitScopeConfig {
    fn default() -> Self {
        Self::identity_default()
    }
}

/// Shared Redis-backed identity and client-IP rate-limit configuration.
///
/// Environment overrides use the standard nested naming convention, for
/// example `<PREFIX>__RATE_LIMIT__REDIS_URL` and
/// `<PREFIX>__RATE_LIMIT__IDENTITY__REQUESTS_PER_PERIOD`.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct RateLimitConfig {
    /// Redis connection URL. Debug and display formatting redact its value.
    pub redis_url: Secret<String>,
    /// Authenticated-principal scope, intentionally lower than the IP net.
    pub identity: RateLimitScopeConfig,
    /// Client-IP safety-net scope.
    #[serde(
        default = "RateLimitScopeConfig::ip_default",
        deserialize_with = "deserialize_ip_rate_limit_scope"
    )]
    pub ip: RateLimitScopeConfig,
    /// Behavior when the backing store cannot make a decision.
    ///
    /// Defaults to [`RateLimitFailMode::Open`], so a store outage disables
    /// limiting. Set `closed` where availability matters less than abuse
    /// protection.
    pub fail_mode: RateLimitFailMode,
    /// Number of trusted reverse-proxy hops, including the socket peer.
    ///
    /// Zero (the default) ignores `X-Forwarded-For` and keys the IP scope on
    /// the socket peer. Set it to the number of proxies in front of the
    /// service that append to `X-Forwarded-For`; any larger value lets
    /// clients choose their own bucket. Deployments outside local that set a
    /// nonzero value must document the ingress topology it reflects.
    pub trusted_proxy_hops: usize,
    /// Prefix length that groups IPv6 clients into one IP-scope bucket.
    ///
    /// Defaults to 64, one bucket per typical end-site subnet. IPv4 and
    /// IPv4-mapped IPv6 addresses always bucket per address.
    pub ipv6_prefix_len: u8,
    /// Prefix prepended to all Redis and in-memory keys.
    pub key_prefix: String,
    /// Tenant namespace the Redis store prepends to every key it touches.
    ///
    /// Empty by default. When set it must end with `:` and contain no
    /// whitespace or Redis glob characters (`*`, `?`, `[`, `]`), so it can be
    /// used verbatim as a Redis ACL key pattern `~<namespace>*`.
    pub redis_key_namespace: String,
}

fn deserialize_ip_rate_limit_scope<'de, D>(
    deserializer: D,
) -> Result<RateLimitScopeConfig, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    struct PartialScope {
        enabled: Option<bool>,
        requests_per_period: Option<u64>,
        period: Option<u64>,
        burst: Option<u64>,
    }

    let partial = PartialScope::deserialize(deserializer)?;
    let defaults = RateLimitScopeConfig::ip_default();
    Ok(RateLimitScopeConfig {
        enabled: partial.enabled.unwrap_or(defaults.enabled),
        requests_per_period: partial
            .requests_per_period
            .unwrap_or(defaults.requests_per_period),
        period: partial.period.map_or(defaults.period, Duration::from_secs),
        burst: partial.burst.unwrap_or(defaults.burst),
    })
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            redis_url: Secret::new("redis://127.0.0.1/".to_owned()),
            identity: RateLimitScopeConfig::identity_default(),
            ip: RateLimitScopeConfig::ip_default(),
            fail_mode: RateLimitFailMode::Open,
            trusted_proxy_hops: 0,
            ipv6_prefix_len: DEFAULT_IPV6_PREFIX_LEN,
            key_prefix: "rl:".to_owned(),
            redis_key_namespace: String::new(),
        }
    }
}

impl Validate for RateLimitConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Vec::new();
        self.identity.validate_into("identity", &mut errors);
        self.ip.validate_into("ip", &mut errors);
        if self.redis_url.expose().trim().is_empty() {
            errors.push(ValidationError::new("redis_url", "must not be empty"));
        }
        if self.key_prefix.trim().is_empty() {
            errors.push(ValidationError::new("key_prefix", "must not be empty"));
        }
        if let Err(message) = validate_redis_key_namespace(&self.redis_key_namespace) {
            errors.push(ValidationError::new("redis_key_namespace", message));
        }
        if self.trusted_proxy_hops > 32 {
            errors.push(ValidationError::new(
                "trusted_proxy_hops",
                "must be at most 32",
            ));
        }
        if !(1..=MAX_IPV6_PREFIX_LEN).contains(&self.ipv6_prefix_len) {
            errors.push(ValidationError::new(
                "ipv6_prefix_len",
                "must be between 1 and 128",
            ));
        }
        validation_result(errors)
    }
}

impl RateLimitConfig {
    /// Validates the configuration together with rules that depend on the deployment.
    ///
    /// Outside [`Environment::Local`], at least one request scope must be
    /// enabled so no deployed service runs without request limiting.
    ///
    /// # Errors
    ///
    /// Returns every problem [`Validate::validate`] finds, plus
    /// `identity.enabled` when both scopes are disabled outside local.
    pub fn validate_in(&self, environment: Environment) -> Result<(), ValidationErrors> {
        let mut errors = self
            .validate()
            .err()
            .map(ValidationErrors::into_errors)
            .unwrap_or_default();
        if environment != Environment::Local && !self.identity.enabled && !self.ip.enabled {
            errors.push(ValidationError::new(
                "identity.enabled",
                "at least one rate-limit scope must be enabled outside local",
            ));
        }
        validation_result(errors)
    }
}
