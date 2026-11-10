use std::{collections::BTreeSet, fmt, net::IpAddr, time::Duration};

use baukit_credential_vault::CredentialCipher;
use baukit_egress::{AddressPolicy, AllowedEndpoint, EgressOptions, EgressOptionsError};
use baukit_http::HttpOptions;
use thiserror::Error;
use url::{Host, Url};
use zeroize::Zeroizing;

use super::rate_limit::{
    AUTH_SESSION_ID_LIMIT, AUTH_SESSION_IP_LIMIT, BrowserSessionRateLimit, BrowserSessionRateLimits,
};

const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const DEFAULT_ABSOLUTE_TIMEOUT: Duration = Duration::from_secs(90 * 24 * 60 * 60);
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const ERROR_BODY_LIMIT: usize = 4096;
const HOST_PREFIX: &str = "__Host-";

/// Configuration for the browser session backend-for-frontend.
///
/// The backend redeems authorization codes and refresh tokens as the same OIDC
/// client the SPA uses, keeps the refresh token encrypted in PostgreSQL, and
/// gives the browser only an `HttpOnly` cookie plus short-lived access tokens.
#[derive(Clone)]
pub struct BrowserSessionConfig {
    pub(crate) issuer: Url,
    pub(crate) issuer_text: String,
    pub(crate) internal_issuer: Option<Url>,
    pub(crate) client_id: String,
    pub(crate) client_secret: Option<Zeroizing<String>>,
    pub(crate) redirect_uris: BTreeSet<String>,
    pub(crate) cookie_name: String,
    pub(crate) public_api_origin: Url,
    pub(crate) cipher: CredentialCipher,
    pub(crate) idle_timeout: Duration,
    pub(crate) absolute_timeout: Duration,
    pub(crate) insecure_local_cookie: bool,
    pub(crate) post_logout_redirect_uri: Option<String>,
    pub(crate) request_timeout: Duration,
    pub(crate) egress_policy: Option<AddressPolicy>,
    pub(crate) rate_limits: BrowserSessionRateLimits,
}

impl BrowserSessionConfig {
    /// Creates configuration for one OIDC client.
    ///
    /// `cookie_name` is the bare name, for example `acme-session`; production
    /// cookies are sent as `__Host-acme-session`. `public_api_origin` is the
    /// origin browsers use to reach this backend.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserSessionConfigError::InvalidIssuer`] when `issuer` is not an
    /// absolute URL, `EmptyClientId` for an empty client ID, `InvalidCookieName` when
    /// `cookie_name` is not `[A-Za-z0-9_-]+`, and `InvalidPublicApiOrigin` when
    /// `public_api_origin` is not an http(s) origin.
    pub fn new(
        issuer: &str,
        client_id: impl Into<String>,
        cookie_name: impl Into<String>,
        public_api_origin: &str,
        cipher: CredentialCipher,
    ) -> Result<Self, BrowserSessionConfigError> {
        let issuer_text = issuer.to_owned();
        let issuer = Url::parse(issuer).map_err(|_| BrowserSessionConfigError::InvalidIssuer)?;
        if issuer.cannot_be_a_base() {
            return Err(BrowserSessionConfigError::InvalidIssuer);
        }
        let client_id = client_id.into();
        if client_id.is_empty() {
            return Err(BrowserSessionConfigError::EmptyClientId);
        }
        let cookie_name = cookie_name.into();
        if !is_cookie_name(&cookie_name) {
            return Err(BrowserSessionConfigError::InvalidCookieName);
        }
        let public_api_origin = parse_origin(public_api_origin)
            .ok_or(BrowserSessionConfigError::InvalidPublicApiOrigin)?;
        Ok(Self {
            issuer,
            issuer_text,
            internal_issuer: None,
            client_id,
            client_secret: None,
            redirect_uris: BTreeSet::new(),
            cookie_name,
            public_api_origin,
            cipher,
            idle_timeout: DEFAULT_IDLE_TIMEOUT,
            absolute_timeout: DEFAULT_ABSOLUTE_TIMEOUT,
            insecure_local_cookie: false,
            post_logout_redirect_uri: None,
            request_timeout: DEFAULT_REQUEST_TIMEOUT,
            egress_policy: None,
            rate_limits: BrowserSessionRateLimits::default(),
        })
    }

    /// Sets the redirect URIs a code exchange may name. At least one is required.
    ///
    /// Each URI is stored in its WHATWG URL serialization, so `http://localhost:5573`
    /// and `http://localhost:5573/` are the same entry. Exchanges compare the
    /// serialized form too; path, query and port must still match. A URI that does
    /// not parse fails [`Self::validate`].
    #[must_use]
    pub fn with_redirect_uris<I, S>(mut self, redirect_uris: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.redirect_uris = redirect_uris
            .into_iter()
            .map(|uri| normalize_or_keep(uri.into()))
            .collect();
        self
    }

    /// Sets the URL this backend uses to reach the IdP's realm when the public
    /// issuer is only reachable from browsers, for example `http://keycloak:8080/realms/app`
    /// next to the public `http://localhost:8081/realms/app` in a container.
    ///
    /// Discovery and every IdP call go to this URL's origin, and the egress policy
    /// allows only that origin. The discovery document must still name the public
    /// issuer, so the IdP needs a fixed hostname and dynamic backchannel URLs.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserSessionConfigError::InvalidIssuer`] when `url` is not an
    /// absolute URL.
    pub fn with_internal_issuer(mut self, url: &str) -> Result<Self, BrowserSessionConfigError> {
        let url = Url::parse(url).map_err(|_| BrowserSessionConfigError::InvalidIssuer)?;
        if url.cannot_be_a_base() {
            return Err(BrowserSessionConfigError::InvalidIssuer);
        }
        self.internal_issuer = Some(url);
        Ok(self)
    }

    /// Sets a client secret for IdPs that require a confidential client.
    #[must_use]
    pub fn with_client_secret(mut self, client_secret: impl Into<String>) -> Self {
        self.client_secret = Some(Zeroizing::new(client_secret.into()));
        self
    }

    /// Sets the idle timeout, the cookie `Max-Age`. Defaults to 30 days.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserSessionConfigError::ZeroTimeout`] when `timeout` is under one second.
    pub fn with_idle_timeout(
        mut self,
        timeout: Duration,
    ) -> Result<Self, BrowserSessionConfigError> {
        if timeout.as_secs() == 0 {
            return Err(BrowserSessionConfigError::ZeroTimeout("idle"));
        }
        self.idle_timeout = timeout;
        Ok(self)
    }

    /// Sets the absolute session lifetime. Defaults to 90 days.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserSessionConfigError::ZeroTimeout`] when `timeout` is under one second.
    pub fn with_absolute_timeout(
        mut self,
        timeout: Duration,
    ) -> Result<Self, BrowserSessionConfigError> {
        if timeout.as_secs() == 0 {
            return Err(BrowserSessionConfigError::ZeroTimeout("absolute"));
        }
        self.absolute_timeout = timeout;
        Ok(self)
    }

    /// Drops the `__Host-` prefix and `Secure` for plain-http local development.
    ///
    /// Startup fails unless every allowed origin and the public API origin are
    /// `http://localhost` or `http://127.0.0.1`.
    #[must_use]
    pub const fn with_insecure_local_cookie(mut self, enabled: bool) -> Self {
        self.insecure_local_cookie = enabled;
        self
    }

    /// Sets `post_logout_redirect_uri` for the IdP end-session URL, in its WHATWG
    /// URL serialization. A URI that does not parse fails [`Self::validate`].
    #[must_use]
    pub fn with_post_logout_redirect_uri(mut self, uri: impl Into<String>) -> Self {
        self.post_logout_redirect_uri = Some(normalize_or_keep(uri.into()));
        self
    }

    /// Sets the timeout for each IdP request. Defaults to 10 seconds.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserSessionConfigError::ZeroTimeout`] when `timeout` is zero.
    pub fn with_request_timeout(
        mut self,
        timeout: Duration,
    ) -> Result<Self, BrowserSessionConfigError> {
        if timeout.is_zero() {
            return Err(BrowserSessionConfigError::ZeroTimeout("request"));
        }
        self.request_timeout = timeout;
        Ok(self)
    }

    /// Overrides the rate limits. Defaults to 30 requests per minute plus a burst
    /// of 10 per client IP and 12 plus 4 per session id (ADR 0006).
    ///
    /// Raise them only for test stacks that sign in many times from one address.
    /// The group names stay `auth-session-ip` and `auth-session-id` whatever
    /// `limits` carries.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserSessionConfigError::ZeroRateLimit`] when a bucket has zero
    /// `requests_per_minute`.
    pub fn with_rate_limits(
        mut self,
        limits: BrowserSessionRateLimits,
    ) -> Result<Self, BrowserSessionConfigError> {
        if limits.client_ip.requests_per_minute == 0 {
            return Err(BrowserSessionConfigError::ZeroRateLimit(
                AUTH_SESSION_IP_LIMIT.group,
            ));
        }
        if limits.session_id.requests_per_minute == 0 {
            return Err(BrowserSessionConfigError::ZeroRateLimit(
                AUTH_SESSION_ID_LIMIT.group,
            ));
        }
        self.rate_limits = BrowserSessionRateLimits {
            client_ip: BrowserSessionRateLimit {
                group: AUTH_SESSION_IP_LIMIT.group,
                ..limits.client_ip
            },
            session_id: BrowserSessionRateLimit {
                group: AUTH_SESSION_ID_LIMIT.group,
                ..limits.session_id
            },
        };
        Ok(self)
    }

    /// Overrides the egress address policy for IdP calls.
    ///
    /// The default allows only the issuer's exact host and port.
    #[must_use]
    pub fn with_egress_policy(mut self, policy: AddressPolicy) -> Self {
        self.egress_policy = Some(policy);
        self
    }

    /// Returns the exact issuer URL.
    #[must_use]
    pub fn issuer(&self) -> &str {
        self.issuer.as_str()
    }

    /// Returns the URL discovery and IdP calls go to: the internal issuer when set.
    pub(crate) fn idp_base(&self) -> &Url {
        self.internal_issuer.as_ref().unwrap_or(&self.issuer)
    }

    /// Returns the cookie name the browser sees, with `__Host-` in production mode.
    #[must_use]
    pub fn cookie_name(&self) -> String {
        if self.insecure_local_cookie {
            self.cookie_name.clone()
        } else {
            format!("{HOST_PREFIX}{}", self.cookie_name)
        }
    }

    pub(crate) fn egress_options(&self) -> Result<EgressOptions, BrowserSessionConfigError> {
        let policy = match &self.egress_policy {
            Some(policy) => policy.clone(),
            None => issuer_only_policy(self.idp_base())?,
        };
        let options = EgressOptions::default()
            .with_policy(policy)
            .with_request_timeout(self.request_timeout)
            .and_then(|options| options.with_error_body_limit(ERROR_BODY_LIMIT))
            .map_err(BrowserSessionConfigError::Egress)?;
        Ok(options)
    }

    /// Checks the configuration against the HTTP options the router runs under.
    ///
    /// Fails when CORS would make the browser drop the cookie, when no redirect
    /// URI is configured, when `insecure_local_cookie` is set for a non-local
    /// origin, or when an allowed origin is not same-site with the public API
    /// origin.
    ///
    /// # Errors
    ///
    /// Returns [`BrowserSessionConfigError::NoRedirectUris`], `InvalidRedirectUri`
    /// naming the first redirect or post-logout URI that is not an absolute URL,
    /// `CredentialsDisabled`, `InvalidAllowedOrigin` or `NoAllowedOrigins` for the
    /// CORS and redirect cases, `InsecureCookieNotLocal` for a non-local insecure
    /// cookie, and `CrossSiteOrigin` naming the first cross-site origin.
    pub fn validate(&self, http: &HttpOptions) -> Result<Vec<Url>, BrowserSessionConfigError> {
        if self.redirect_uris.is_empty() {
            return Err(BrowserSessionConfigError::NoRedirectUris);
        }
        if let Some(invalid) = self
            .redirect_uris
            .iter()
            .chain(&self.post_logout_redirect_uri)
            .find(|uri| normalize_uri(uri).is_none())
        {
            return Err(BrowserSessionConfigError::InvalidRedirectUri(
                invalid.clone(),
            ));
        }
        if !http.allow_credentials {
            return Err(BrowserSessionConfigError::CredentialsDisabled);
        }
        let origins = http
            .allowed_origins()
            .iter()
            .map(|value| {
                value
                    .to_str()
                    .ok()
                    .and_then(parse_origin)
                    .ok_or(BrowserSessionConfigError::InvalidAllowedOrigin)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if origins.is_empty() {
            return Err(BrowserSessionConfigError::NoAllowedOrigins);
        }
        if self.insecure_local_cookie
            && !(is_local_http(&self.public_api_origin) && origins.iter().all(is_local_http))
        {
            return Err(BrowserSessionConfigError::InsecureCookieNotLocal);
        }
        if let Some(origin) = origins
            .iter()
            .find(|origin| !is_same_site(origin, &self.public_api_origin))
        {
            return Err(BrowserSessionConfigError::CrossSiteOrigin(
                origin.origin().ascii_serialization(),
            ));
        }
        Ok(origins)
    }
}

impl fmt::Debug for BrowserSessionConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BrowserSessionConfig")
            .field("issuer", &self.issuer.as_str())
            .field("client_id", &self.client_id)
            .field(
                "client_secret",
                &self.client_secret.as_ref().map(|_| "[redacted]"),
            )
            .field("redirect_uris", &self.redirect_uris)
            .field("cookie_name", &self.cookie_name)
            .field("public_api_origin", &self.public_api_origin.as_str())
            .field("idle_timeout", &self.idle_timeout)
            .field("absolute_timeout", &self.absolute_timeout)
            .field("insecure_local_cookie", &self.insecure_local_cookie)
            .finish_non_exhaustive()
    }
}

/// Invalid browser session configuration. Startup must fail on any variant.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum BrowserSessionConfigError {
    /// The issuer is not an absolute hierarchical URL.
    #[error("browser session issuer must be an absolute URL")]
    InvalidIssuer,
    /// The client id is empty.
    #[error("browser session client id must not be empty")]
    EmptyClientId,
    /// The cookie name is empty or contains characters outside `[A-Za-z0-9_-]`.
    #[error("browser session cookie name must match [A-Za-z0-9_-]+")]
    InvalidCookieName,
    /// The public API origin is not an http(s) origin.
    #[error("browser session public API origin must be an http(s) origin")]
    InvalidPublicApiOrigin,
    /// A timeout is zero.
    #[error("browser session {0} timeout must be greater than zero")]
    ZeroTimeout(&'static str),
    /// A rate limit bucket has zero requests per minute.
    #[error("browser session rate limit {0} must allow at least one request per minute")]
    ZeroRateLimit(&'static str),
    /// No redirect URI is configured.
    #[error("browser session needs at least one redirect URI")]
    NoRedirectUris,
    /// A redirect or post-logout URI is not an absolute URL.
    #[error("browser session redirect URI {0:?} is not an absolute URL")]
    InvalidRedirectUri(String),
    /// `HttpOptions.allowed_origins` is empty.
    #[error("browser session needs at least one allowed CORS origin")]
    NoAllowedOrigins,
    /// `HttpOptions.allow_credentials` is off.
    #[error("browser session needs CORS allow_credentials")]
    CredentialsDisabled,
    /// An allowed origin is not an http(s) origin.
    #[error("an allowed CORS origin is not an http(s) origin")]
    InvalidAllowedOrigin,
    /// `insecure_local_cookie` is set while an origin is not local plain http.
    #[error("insecure_local_cookie requires http://localhost or http://127.0.0.1 origins only")]
    InsecureCookieNotLocal,
    /// An allowed origin does not share the public API origin's registrable domain.
    #[error("allowed origin {0} is not same-site with the public API origin")]
    CrossSiteOrigin(String),
    /// The issuer host cannot form the default egress allowance.
    #[error("issuer host cannot be used as an egress endpoint")]
    IssuerEndpoint,
    /// The egress options are invalid.
    #[error("invalid IdP egress options: {0}")]
    Egress(#[source] EgressOptionsError),
}

/// Returns the WHATWG serialization of `uri`, or `None` when it does not parse.
pub(crate) fn normalize_uri(uri: &str) -> Option<String> {
    Url::parse(uri).ok().map(String::from)
}

fn normalize_or_keep(uri: String) -> String {
    normalize_uri(&uri).unwrap_or(uri)
}

fn issuer_only_policy(issuer: &Url) -> Result<AddressPolicy, BrowserSessionConfigError> {
    let host = issuer
        .host_str()
        .ok_or(BrowserSessionConfigError::IssuerEndpoint)?;
    let port = issuer
        .port_or_known_default()
        .ok_or(BrowserSessionConfigError::IssuerEndpoint)?;
    let endpoint = AllowedEndpoint::new(host, port, issuer.scheme() == "http")
        .map_err(|_| BrowserSessionConfigError::IssuerEndpoint)?;
    Ok(AddressPolicy::OnlyEndpoint(endpoint))
}

fn is_cookie_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

pub(crate) fn parse_origin(value: &str) -> Option<Url> {
    let url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host().is_none() {
        return None;
    }
    Url::parse(&url.origin().ascii_serialization()).ok()
}

fn is_local_http(origin: &Url) -> bool {
    origin.scheme() == "http"
        && match origin.host() {
            Some(Host::Domain(domain)) => domain == "localhost",
            Some(Host::Ipv4(address)) => IpAddr::V4(address).is_loopback(),
            Some(Host::Ipv6(_)) | None => false,
        }
}

/// Returns whether two origins are schemefully same-site.
///
/// Domains compare by registrable domain from the public suffix list. IP
/// addresses and hosts without a registrable domain, such as `localhost`, must
/// match exactly. Ports never matter.
pub(crate) fn is_same_site(left: &Url, right: &Url) -> bool {
    if left.scheme() != right.scheme() {
        return false;
    }
    match (left.host(), right.host()) {
        (Some(Host::Domain(left)), Some(Host::Domain(right))) => {
            match (psl::domain_str(left), psl::domain_str(right)) {
                (Some(left), Some(right)) => left == right,
                _ => left == right,
            }
        }
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}
