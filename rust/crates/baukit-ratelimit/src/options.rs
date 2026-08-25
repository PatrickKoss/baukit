use std::{fmt, net::IpAddr};

use baukit_config::{RateLimitConfig, RateLimitFailMode, RateLimitScopeConfig, Validate as _};

use crate::{Quota, QuotaError};

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
        config
            .validate()
            .map_err(RateLimitOptionsError::Configuration)?;
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
}
