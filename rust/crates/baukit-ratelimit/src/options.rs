use std::{fmt, net::IpAddr};

use baukit_config::{
    RateLimitConfig, RateLimitFailMode, RateLimitScopeConfig, Validate as _, ValidationErrors,
};

use crate::{Quota, QuotaError};

const MAX_ROUTE_GROUP_NAME_LENGTH: usize = 64;

/// Validated options for one request scope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RateLimitScopeOptions {
    /// Whether requests are checked in this scope.
    pub enabled: bool,
    /// Token-bucket quota used when the scope is enabled.
    pub quota: Quota,
}

/// Validated rate-limit middleware and adapter options.
#[derive(Clone)]
pub struct RateLimitOptions {
    redis_url: String,
    /// Authenticated-principal policy.
    pub identity: RateLimitScopeOptions,
    /// Client-IP safety-net policy.
    pub ip: RateLimitScopeOptions,
    /// Behavior when the store cannot make a decision.
    pub fail_mode: RateLimitFailMode,
    /// Number of trusted reverse-proxy hops, including the socket peer.
    pub trusted_proxy_hops: usize,
    key_prefix: String,
}

impl RateLimitOptions {
    /// Converts the shared application configuration into validated runtime options.
    pub fn from_config(config: &RateLimitConfig) -> Result<Self, RateLimitOptionsError> {
        validate_config(config)?;
        Ok(Self {
            redis_url: config.redis_url.expose().clone(),
            identity: scope_options(&config.identity)?,
            ip: scope_options(&config.ip)?,
            fail_mode: config.fail_mode,
            trusted_proxy_hops: config.trusted_proxy_hops,
            key_prefix: config.key_prefix.clone(),
        })
    }

    /// Returns the Redis connection URL.
    #[must_use]
    pub fn redis_url(&self) -> &str {
        &self.redis_url
    }

    /// Returns whether either request scope needs a store.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.identity.enabled || self.ip.enabled
    }

    /// Returns the configured key prefix.
    #[must_use]
    pub fn key_prefix(&self) -> &str {
        &self.key_prefix
    }

    /// Derives the fully scoped store key for an authenticated subject.
    #[must_use]
    pub fn identity_key(&self, subject: &str) -> String {
        format!("{}id:{subject}", self.key_prefix)
    }

    /// Derives the fully scoped store key for a client IP address.
    #[must_use]
    pub fn ip_key(&self, address: Option<IpAddr>) -> String {
        match address {
            Some(address) => format!("{}ip:{address}", self.key_prefix),
            None => format!("{}ip:unknown", self.key_prefix),
        }
    }
}

impl Default for RateLimitOptions {
    fn default() -> Self {
        Self::from_config(&RateLimitConfig::default())
            .expect("the standard rate-limit defaults are valid")
    }
}

impl fmt::Debug for RateLimitOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RateLimitOptions")
            .field("redis_url", &"[redacted]")
            .field("identity", &self.identity)
            .field("ip", &self.ip)
            .field("fail_mode", &self.fail_mode)
            .field("trusted_proxy_hops", &self.trusted_proxy_hops)
            .field("key_prefix", &self.key_prefix)
            .finish()
    }
}

/// Validated options for one authenticated route group.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticatedRouteGroupOptions {
    group: String,
    /// Token-bucket quota for this group and subject key.
    pub quota: Quota,
    /// Behavior when the store cannot make a decision.
    pub fail_mode: RateLimitFailMode,
    key_prefix: String,
}

impl AuthenticatedRouteGroupOptions {
    /// Creates a route-group policy that shares the global key prefix and fail mode.
    pub fn new(
        group: impl Into<String>,
        quota: Quota,
        rate_limit: &RateLimitOptions,
    ) -> Result<Self, AuthenticatedRouteGroupOptionsError> {
        let group = group.into();
        validate_route_group_name(&group)?;
        Ok(Self {
            group,
            quota,
            fail_mode: rate_limit.fail_mode,
            key_prefix: rate_limit.key_prefix.clone(),
        })
    }

    /// Returns the bounded group name used as a metric label.
    #[must_use]
    pub fn group(&self) -> &str {
        &self.group
    }

    pub(crate) fn key(&self, subject_key: &str) -> String {
        format!("{}group:{}:{subject_key}", self.key_prefix, self.group)
    }
}

/// Invalid authenticated route-group options.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum AuthenticatedRouteGroupOptionsError {
    /// A group name is required for the counter namespace and metric label.
    #[error("authenticated route-group name must not be empty")]
    EmptyGroup,
    /// Group names are bounded to keep counter keys and metric labels small.
    #[error("authenticated route-group name must be at most 64 bytes")]
    GroupTooLong,
    /// Group names use a restricted ASCII character set.
    #[error(
        "authenticated route-group name may contain only ASCII letters, digits, `_`, `-`, and `.`"
    )]
    InvalidGroup,
}

fn validate_route_group_name(group: &str) -> Result<(), AuthenticatedRouteGroupOptionsError> {
    if group.is_empty() {
        return Err(AuthenticatedRouteGroupOptionsError::EmptyGroup);
    }
    if group.len() > MAX_ROUTE_GROUP_NAME_LENGTH {
        return Err(AuthenticatedRouteGroupOptionsError::GroupTooLong);
    }
    if !group
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(AuthenticatedRouteGroupOptionsError::InvalidGroup);
    }
    Ok(())
}

/// Failure while converting rate-limit configuration into runtime options.
#[derive(Debug, thiserror::Error)]
pub enum RateLimitOptionsError {
    /// Shared configuration invariants were violated.
    #[error(transparent)]
    Configuration(#[from] baukit_config::ValidationErrors),
    /// One scope's token-bucket quota was invalid.
    #[error(transparent)]
    Quota(#[from] QuotaError),
}

fn scope_options(config: &RateLimitScopeConfig) -> Result<RateLimitScopeOptions, QuotaError> {
    // Disabled scopes still retain a well-formed quota so enabling them does not
    // require reconstructing the middleware. Validation permits zero values for
    // disabled scopes, which map to the smallest valid dormant quota here.
    if !config.enabled {
        return Ok(RateLimitScopeOptions {
            enabled: false,
            quota: Quota::new(1, std::time::Duration::from_secs(1), 0)?,
        });
    }
    Ok(RateLimitScopeOptions {
        enabled: true,
        quota: Quota::new(config.requests_per_period, config.period, config.burst)?,
    })
}

fn validate_config(config: &RateLimitConfig) -> Result<(), RateLimitOptionsError> {
    let Err(errors) = config.validate() else {
        return Ok(());
    };
    if config.identity.enabled || config.ip.enabled {
        return Err(RateLimitOptionsError::Configuration(errors));
    }
    let errors = errors
        .into_errors()
        .into_iter()
        .filter(|error| error.path() != "redis_url")
        .collect::<Vec<_>>();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(RateLimitOptionsError::Configuration(ValidationErrors::new(
            errors,
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use baukit_config::{RateLimitConfig, Secret};

    use super::*;

    #[test]
    fn defaults_are_valid_and_identity_is_lower_than_ip() {
        let options = RateLimitOptions::default();
        assert!(options.identity.quota.capacity() < options.ip.quota.capacity());
        assert_eq!(options.trusted_proxy_hops, 1);
        assert_eq!(options.key_prefix(), "rl:");
    }

    #[test]
    fn invalid_enabled_scope_and_prefix_are_rejected() {
        let mut config = RateLimitConfig::default();
        config.identity.requests_per_period = 0;
        config.identity.period = Duration::ZERO;
        config.redis_url = Secret::new(String::new());
        config.key_prefix.clear();
        let error = RateLimitOptions::from_config(&config).expect_err("invalid options");
        let rendered = error.to_string();
        assert!(rendered.contains("identity.requests_per_period"));
        assert!(rendered.contains("identity.period"));
        assert!(rendered.contains("redis_url"));
        assert!(rendered.contains("key_prefix"));
    }

    #[test]
    fn keys_separate_identity_and_ip_namespaces() {
        let options = RateLimitOptions::default();
        assert_eq!(options.identity_key("alice"), "rl:id:alice");
        assert_eq!(
            options.ip_key(Some("2001:db8::1".parse().expect("IP"))),
            "rl:ip:2001:db8::1"
        );
    }

    #[test]
    fn disabled_scopes_do_not_require_a_redis_url() {
        let mut config = RateLimitConfig::default();
        config.identity.enabled = false;
        config.ip.enabled = false;
        config.redis_url = Secret::new(String::new());

        let options = RateLimitOptions::from_config(&config).expect("disabled options");

        assert!(!options.is_enabled());
        assert!(options.redis_url().is_empty());
    }

    #[test]
    fn route_group_names_are_bounded_and_safe_for_keys_and_labels() {
        let options = RateLimitOptions::default();
        let quota = Quota::new(1, Duration::from_secs(60), 0).expect("quota");
        let group = AuthenticatedRouteGroupOptions::new("sync_push.v2", quota, &options)
            .expect("valid group");
        assert_eq!(group.group(), "sync_push.v2");
        assert_eq!(group.key("subject"), "rl:group:sync_push.v2:subject");

        for (name, expected) in [
            ("", AuthenticatedRouteGroupOptionsError::EmptyGroup),
            (
                &"a".repeat(MAX_ROUTE_GROUP_NAME_LENGTH + 1),
                AuthenticatedRouteGroupOptionsError::GroupTooLong,
            ),
            (
                "sync/push",
                AuthenticatedRouteGroupOptionsError::InvalidGroup,
            ),
            (
                "sync:push",
                AuthenticatedRouteGroupOptionsError::InvalidGroup,
            ),
            (
                "sync push",
                AuthenticatedRouteGroupOptionsError::InvalidGroup,
            ),
        ] {
            assert_eq!(
                AuthenticatedRouteGroupOptions::new(name, quota, &options)
                    .expect_err("unsafe group name"),
                expected
            );
        }
    }
}
