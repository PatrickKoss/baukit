use std::{error::Error, sync::Arc, time::Duration};

use axum::Router;
use baukit_auth::browser_session::{
    self, AUTH_SESSION_ID_LIMIT, AUTH_SESSION_IP_LIMIT, BrowserSessionConfig,
    BrowserSessionRateLimit, BrowserSessionRateLimiter, BrowserSessionRateLimits,
    BrowserSessionState,
};
use baukit_config::{Environment, Secret, Validate, ValidationError, ValidationErrors};
use baukit_credential_vault::CredentialCipher;
use baukit_http::HttpOptions;
use baukit_ratelimit::{
    InMemoryRateLimitStore, RateLimitOptions, RedisRateLimitStore, StoreBrowserSessionRateLimiter,
};
use serde::Deserialize;
use sqlx::PgPool;

const PRODUCT: &str = "{{ context.app_name }}";
const SECONDS_PER_DAY: u64 = 24 * 60 * 60;
const DEFAULT_IDLE_TIMEOUT_DAYS: u64 = 30;
const DEFAULT_ABSOLUTE_TIMEOUT_DAYS: u64 = 90;
const LOCAL_WEB_PORTS: &[u16] = &[{{ context.web_origin_ports | join(", ") }}];
const LOCAL_CREDENTIAL_KEYS: &str = "1:bG9jYWwtYnJvd3Nlci1zZXNzaW9uLWtleS1ub3Qtb2s=";

/// Backend-for-frontend that keeps web refresh tokens server side (ADR 0006).
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct BrowserSessionSettings {
    pub enabled: bool,
    pub client_id: String,
    pub client_secret: Option<Secret<String>>,
    /// Realm URL the backend calls when the public issuer is only reachable from browsers.
    pub internal_issuer: Option<String>,
    pub redirect_uris: Vec<String>,
    pub post_logout_redirect_uri: Option<String>,
    pub cookie_name: String,
    pub public_api_origin: String,
    pub idle_timeout_seconds: u64,
    pub absolute_timeout_seconds: u64,
    pub insecure_local_cookie: bool,
    /// `version:base64key,...` keyring that encrypts stored refresh tokens.
    pub credential_keys: Option<Secret<String>>,
    /// Per client IP bucket. Defaults to the ADR 0006 limit; raise it only in test stacks.
    pub ip_rate_per_minute: u64,
    pub ip_rate_burst: u64,
    /// Per session id bucket. Defaults to the ADR 0006 limit; raise it only in test stacks.
    pub session_rate_per_minute: u64,
    pub session_rate_burst: u64,
}

impl Default for BrowserSessionSettings {
    fn default() -> Self {
        Self {
            enabled: {{ "true" if (context.web or context.pwa) and context.auth_oidc else "false" }},
            client_id: format!("{PRODUCT}-web"),
            client_secret: None,
            internal_issuer: None,
            redirect_uris: LOCAL_WEB_PORTS
                .iter()
                .map(|port| format!("http://localhost:{port}/"))
                .collect(),
            post_logout_redirect_uri: Some("http://localhost:{{ context.web_port }}/".to_owned()),
            cookie_name: format!("{PRODUCT}-session"),
            public_api_origin: "http://localhost:{{ context.api_host_port }}".to_owned(),
            idle_timeout_seconds: DEFAULT_IDLE_TIMEOUT_DAYS * SECONDS_PER_DAY,
            absolute_timeout_seconds: DEFAULT_ABSOLUTE_TIMEOUT_DAYS * SECONDS_PER_DAY,
            insecure_local_cookie: false,
            credential_keys: None,
            ip_rate_per_minute: AUTH_SESSION_IP_LIMIT.requests_per_minute,
            ip_rate_burst: AUTH_SESSION_IP_LIMIT.burst,
            session_rate_per_minute: AUTH_SESSION_ID_LIMIT.requests_per_minute,
            session_rate_burst: AUTH_SESSION_ID_LIMIT.burst,
        }
    }
}

impl BrowserSessionSettings {
    fn rate_limits(&self) -> BrowserSessionRateLimits {
        let defaults = BrowserSessionRateLimits::default();
        BrowserSessionRateLimits {
            client_ip: BrowserSessionRateLimit {
                requests_per_minute: self.ip_rate_per_minute,
                burst: self.ip_rate_burst,
                ..defaults.client_ip
            },
            session_id: BrowserSessionRateLimit {
                requests_per_minute: self.session_rate_per_minute,
                burst: self.session_rate_burst,
                ..defaults.session_id
            },
        }
    }
}

impl Validate for BrowserSessionSettings {
    fn validate(&self) -> Result<(), ValidationErrors> {
        if !self.enabled {
            return Ok(());
        }
        let mut errors = Vec::new();
        if self.redirect_uris.is_empty() {
            errors.push(ValidationError::new(
                "auth.browser_session.redirect_uris",
                "must name the web callback URL",
            ));
        }
        if self.idle_timeout_seconds == 0 {
            errors.push(ValidationError::new(
                "auth.browser_session.idle_timeout_seconds",
                "must be greater than zero",
            ));
        }
        if self.absolute_timeout_seconds < self.idle_timeout_seconds {
            errors.push(ValidationError::new(
                "auth.browser_session.absolute_timeout_seconds",
                "must not be shorter than the idle timeout",
            ));
        }
        for (field, value) in [
            ("ip_rate_per_minute", self.ip_rate_per_minute),
            ("session_rate_per_minute", self.session_rate_per_minute),
        ] {
            if value == 0 {
                errors.push(ValidationError::new(
                    format!("auth.browser_session.{field}"),
                    "must be greater than zero",
                ));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors::new(errors))
        }
    }
}

/// Inputs for [`browser_session_router`] that come from the rest of the API composition.
pub struct BrowserSessionDependencies<'a> {
    pub issuer: &'a str,
    pub http: &'a HttpOptions,
    pub pool: PgPool,
    pub rate_limit: &'a RateLimitOptions,
    pub rate_limit_store: Option<RedisRateLimitStore>,
    pub environment: Environment,
}

/// Builds `/auth/session/*`, or `None` when the product does not enable it.
pub async fn browser_session_router(
    settings: &BrowserSessionSettings,
    dependencies: BrowserSessionDependencies<'_>,
) -> Result<Option<Router>, Box<dyn Error>> {
    if !settings.enabled {
        return Ok(None);
    }
    let local = dependencies.environment == Environment::Local;
    let keys = match &settings.credential_keys {
        Some(keys) => keys.expose().clone(),
        None if local => LOCAL_CREDENTIAL_KEYS.to_owned(),
        None => {
            return Err(
                "auth.browser_session.credential_keys must be configured outside local development"
                    .into(),
            );
        }
    };
    let mut config = BrowserSessionConfig::new(
        dependencies.issuer,
        settings.client_id.clone(),
        settings.cookie_name.clone(),
        &settings.public_api_origin,
        CredentialCipher::parse(&keys)?,
    )?
    .with_redirect_uris(settings.redirect_uris.iter().cloned())
    .with_idle_timeout(Duration::from_secs(settings.idle_timeout_seconds))?
    .with_absolute_timeout(Duration::from_secs(settings.absolute_timeout_seconds))?
    .with_insecure_local_cookie(settings.insecure_local_cookie)
    .with_rate_limits(settings.rate_limits())?;
    if let Some(internal_issuer) = &settings.internal_issuer {
        config = config.with_internal_issuer(internal_issuer)?;
    }
    if let Some(secret) = &settings.client_secret {
        config = config.with_client_secret(secret.expose().clone());
    }
    if let Some(uri) = &settings.post_logout_redirect_uri {
        config = config.with_post_logout_redirect_uri(uri.clone());
    }
    let limiter = rate_limiter(
        dependencies.rate_limit_store,
        dependencies.rate_limit,
        local,
    )?;
    let state =
        BrowserSessionState::discover(config, dependencies.http, dependencies.pool, limiter)
            .await?;
    Ok(Some(browser_session::router(state)))
}

fn rate_limiter(
    store: Option<RedisRateLimitStore>,
    options: &RateLimitOptions,
    local: bool,
) -> Result<Arc<dyn BrowserSessionRateLimiter>, Box<dyn Error>> {
    match store {
        Some(store) => Ok(Arc::new(StoreBrowserSessionRateLimiter::new(
            store,
            options.clone(),
        ))),
        None if local => Ok(Arc::new(StoreBrowserSessionRateLimiter::new(
            InMemoryRateLimitStore::default(),
            options.clone(),
        ))),
        None => Err(
            "browser sessions require the shared rate-limit store outside local development".into(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled() -> BrowserSessionSettings {
        BrowserSessionSettings {
            enabled: true,
            ..BrowserSessionSettings::default()
        }
    }

    #[test]
    fn rate_limits_default_to_the_adr_values() {
        assert_eq!(enabled().rate_limits(), BrowserSessionRateLimits::default());
    }

    #[test]
    fn rate_limit_overrides_replace_both_buckets() {
        let limits = BrowserSessionSettings {
            ip_rate_per_minute: 6_000,
            ip_rate_burst: 500,
            session_rate_per_minute: 1_200,
            session_rate_burst: 100,
            ..enabled()
        }
        .rate_limits();
        assert_eq!(limits.client_ip.requests_per_minute, 6_000);
        assert_eq!(limits.client_ip.burst, 500);
        assert_eq!(limits.session_id.requests_per_minute, 1_200);
        assert_eq!(limits.session_id.burst, 100);
    }

    #[test]
    fn zero_rate_per_minute_fails_validation() {
        for settings in [
            BrowserSessionSettings {
                ip_rate_per_minute: 0,
                ..enabled()
            },
            BrowserSessionSettings {
                session_rate_per_minute: 0,
                ..enabled()
            },
        ] {
            assert!(settings.validate().is_err());
        }
    }
}
