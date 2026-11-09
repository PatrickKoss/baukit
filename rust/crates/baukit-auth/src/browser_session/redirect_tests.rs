use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use baukit_credential_vault::CredentialCipher;
use baukit_http::HttpOptions;

use super::{
    AUTH_SESSION_ID_LIMIT, AUTH_SESSION_IP_LIMIT, BrowserSessionConfig, BrowserSessionConfigError,
    BrowserSessionRateLimit, BrowserSessionRateLimits, service::SessionSettings,
};

type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

const ORIGIN: &str = "http://localhost:5573";

fn cipher() -> Result<CredentialCipher, Box<dyn std::error::Error + Send + Sync>> {
    Ok(CredentialCipher::parse(&format!(
        "1:{}",
        STANDARD.encode([9_u8; 32])
    ))?)
}

fn config(
    redirect_uri: &str,
) -> Result<BrowserSessionConfig, Box<dyn std::error::Error + Send + Sync>> {
    Ok(BrowserSessionConfig::new(
        "http://localhost:8180/realms/test",
        "web",
        "acme-session",
        "http://localhost:8080",
        cipher()?,
    )?
    .with_insecure_local_cookie(true)
    .with_redirect_uris([redirect_uri]))
}

fn settings(
    redirect_uri: &str,
) -> Result<SessionSettings, Box<dyn std::error::Error + Send + Sync>> {
    let config = config(redirect_uri)?;
    Ok(SessionSettings {
        cipher: config.cipher,
        redirect_uris: config.redirect_uris,
        idle_timeout: Duration::from_secs(60),
        absolute_timeout: Duration::from_secs(120),
    })
}

fn http_options() -> Result<HttpOptions, Box<dyn std::error::Error + Send + Sync>> {
    let mut options = HttpOptions::default().with_allowed_origins([ORIGIN])?;
    options.allow_credentials = true;
    Ok(options)
}

#[test]
fn origin_without_trailing_slash_accepts_the_normalized_form() -> TestResult {
    let settings = settings(ORIGIN)?;
    assert!(settings.accepts_redirect_uri("http://localhost:5573/"));
    assert!(settings.accepts_redirect_uri("HTTP://LOCALHOST:5573"));
    Ok(())
}

#[test]
fn origin_with_trailing_slash_accepts_the_bare_origin() -> TestResult {
    let settings = settings("http://localhost:5573/")?;
    assert!(settings.accepts_redirect_uri(ORIGIN));
    Ok(())
}

#[test]
fn path_query_and_port_must_still_match() -> TestResult {
    let settings = settings(ORIGIN)?;
    assert!(!settings.accepts_redirect_uri("http://localhost:5573/other"));
    assert!(!settings.accepts_redirect_uri("http://localhost:5573/?a=1"));
    assert!(!settings.accepts_redirect_uri("http://localhost:5574/"));
    assert!(!settings.accepts_redirect_uri("not a url"));
    Ok(())
}

#[test]
fn invalid_configured_uris_fail_validation() -> TestResult {
    let result = config("/auth/callback")?.validate(&http_options()?);
    assert!(matches!(
        result,
        Err(BrowserSessionConfigError::InvalidRedirectUri(uri)) if uri == "/auth/callback"
    ));
    let result = config(ORIGIN)?
        .with_post_logout_redirect_uri("relative")
        .validate(&http_options()?);
    assert!(matches!(
        result,
        Err(BrowserSessionConfigError::InvalidRedirectUri(uri)) if uri == "relative"
    ));
    assert!(config(ORIGIN)?.validate(&http_options()?).is_ok());
    Ok(())
}

#[test]
fn default_rate_limits_equal_the_documented_constants() -> TestResult {
    let limits = config(ORIGIN)?.rate_limits;
    assert_eq!(limits.client_ip, AUTH_SESSION_IP_LIMIT);
    assert_eq!(limits.session_id, AUTH_SESSION_ID_LIMIT);
    Ok(())
}

#[test]
fn zero_requests_per_minute_is_rejected_for_either_bucket() -> TestResult {
    let defaults = BrowserSessionRateLimits::default();
    let zero_ip = BrowserSessionRateLimits {
        client_ip: BrowserSessionRateLimit {
            requests_per_minute: 0,
            ..defaults.client_ip
        },
        ..defaults
    };
    let zero_session = BrowserSessionRateLimits {
        session_id: BrowserSessionRateLimit {
            requests_per_minute: 0,
            ..defaults.session_id
        },
        ..defaults
    };
    assert!(matches!(
        config(ORIGIN)?.with_rate_limits(zero_ip),
        Err(BrowserSessionConfigError::ZeroRateLimit("auth-session-ip"))
    ));
    assert!(matches!(
        config(ORIGIN)?.with_rate_limits(zero_session),
        Err(BrowserSessionConfigError::ZeroRateLimit("auth-session-id"))
    ));
    Ok(())
}

#[test]
fn custom_rate_limits_are_stored_with_fixed_groups() -> TestResult {
    let custom = BrowserSessionRateLimits {
        client_ip: BrowserSessionRateLimit {
            group: "other",
            requests_per_minute: 1_000,
            burst: 0,
        },
        ..BrowserSessionRateLimits::default()
    };
    let limits = config(ORIGIN)?.with_rate_limits(custom)?.rate_limits;
    assert_eq!(limits.client_ip.group, AUTH_SESSION_IP_LIMIT.group);
    assert_eq!(limits.client_ip.requests_per_minute, 1_000);
    assert_eq!(limits.session_id, AUTH_SESSION_ID_LIMIT);
    Ok(())
}
