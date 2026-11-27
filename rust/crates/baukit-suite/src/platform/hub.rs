use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use baukit_config::{Secret, Validate, ValidationError, ValidationErrors};
use baukit_egress::{EgressError, EgressRequest, EgressResponse, GuardedClient};
use baukit_resilience::{
    CacheRead, Clock, Freshness, StaleCache, StaleCacheError, StalePolicy, SystemClock,
};
use serde::{Deserialize, de::DeserializeOwned};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use url::Url;

pub use super::hub_request::HubRequest;
use super::hub_request::Replay;

/// Tokens are replaced this long before `expires_in` elapses, or at half their
/// lifetime when that is shorter.
pub const TOKEN_REFRESH_MARGIN: Duration = Duration::from_secs(30);
/// Minimum gap between token requests after a failed one.
pub const TOKEN_FAILURE_COOLDOWN: Duration = Duration::from_secs(5);
/// Upper bound for a token's `expires_in`, and the cache's fresh window.
const MAX_TOKEN_LIFETIME: Duration = Duration::from_secs(24 * 60 * 60);
const CLIENT_ID_PREFIX: &str = "platform-";
const IDEMPOTENCY_KEY: HeaderName = HeaderName::from_static("idempotency-key");
const JSON: &str = "application/json";
const FORM: &str = "application/x-www-form-urlencoded";

/// Service credentials for the hub's `/platform/v1` API. Embed it next to
/// `PlatformConfig`, for example as `platform_client`. Set either `token` or
/// `token_url` with `client_secret`, never both.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct HubClientConfig {
    /// Hub API base URL. Requests go to `<api_url>/platform/v1/...`.
    pub api_url: String,
    /// A hub-issued `sbsvc_` platform service token, sent as the bearer on
    /// every request. It is never refreshed.
    pub token: Option<Secret<String>>,
    /// The hub realm's OAuth token endpoint for client credentials.
    #[serde(deserialize_with = "crate::config::empty_string_as_none")]
    pub token_url: Option<String>,
    /// Defaults to `platform-<product_id>`.
    #[serde(deserialize_with = "crate::config::empty_string_as_none")]
    pub client_id: Option<String>,
    pub client_secret: Option<Secret<String>>,
}
impl HubClientConfig {
    fn static_token(&self) -> Option<&Secret<String>> {
        self.token
            .as_ref()
            .filter(|token| !token.expose().is_empty())
    }
    fn uses_client_credentials(&self) -> bool {
        self.token_url.is_some() || self.client_id.is_some() || self.client_secret.is_some()
    }
}
impl Validate for HubClientConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Vec::new();
        if base_url(&self.api_url).is_none() {
            errors.push(ValidationError::new("platform_client.api_url", URL_RULE));
        }
        match (self.static_token(), self.uses_client_credentials()) {
            (Some(_), true) => errors.push(ValidationError::new(
                "platform_client",
                "set either token or the client credentials, not both",
            )),
            (None, false) => errors.push(ValidationError::new(
                "platform_client",
                "requires token or the client credentials",
            )),
            (Some(token), false) => {
                if token.expose().chars().any(invalid_token_char) {
                    errors.push(ValidationError::new(
                        "platform_client.token",
                        "must not contain whitespace or control characters",
                    ));
                }
            }
            (None, true) => validate_client_credentials(self, &mut errors),
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors::new(errors))
        }
    }
}

const URL_RULE: &str = "must be an HTTP(S) URL without credentials, query or fragment";

fn invalid_token_char(character: char) -> bool {
    character.is_whitespace() || character.is_control()
}

fn validate_client_credentials(config: &HubClientConfig, errors: &mut Vec<ValidationError>) {
    if config.token_url.as_deref().and_then(base_url).is_none() {
        errors.push(ValidationError::new("platform_client.token_url", URL_RULE));
    }
    if config
        .client_id
        .as_deref()
        .is_some_and(|id| id.trim() != id || id.chars().any(char::is_control) || id.is_empty())
    {
        errors.push(ValidationError::new(
            "platform_client.client_id",
            "must not contain whitespace padding or control characters",
        ));
    }
    if !config
        .client_secret
        .as_ref()
        .is_some_and(|secret| !secret.expose().is_empty())
    {
        errors.push(ValidationError::new(
            "platform_client.client_secret",
            "is required",
        ));
    }
}

/// A hub call that did not return a usable response. No variant carries a URL,
/// token or secret.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum HubError {
    /// Transport failure, timeout, rate limit, or a 408 or 5xx response.
    #[error("hub_unavailable")]
    Unavailable,
    /// The egress breaker is open, the client is saturated, or token requests
    /// are cooling down after a failure. No request was sent.
    #[error("hub_unavailable")]
    CoolingDown { retry_after: Duration },
    /// The token endpoint refused the credentials, the API returned 401
    /// again with a freshly issued token, or it returned 401 for the static
    /// service token.
    #[error("hub_unauthorized")]
    Unauthorized,
    /// A 2xx response whose body does not match the contract.
    #[error("hub_response_invalid")]
    InvalidResponse,
    /// Any other status, with the Baukit error envelope code when present.
    #[error("hub_rejected")]
    Rejected { status: u16, code: Option<String> },
}

/// Encodes every byte except ASCII alphanumerics and `*-._`, so `|` in a
/// suite subject becomes `%7C`.
const DOT_SEGMENT_CODE: &str = "hub_path_segment_invalid";

/// Percent-encodes one path segment. `None` for `.` and `..`: URL parsers resolve
/// them, encoded or not, as dot segments that would leave `/platform/v1/`.
fn encode_segment(segment: &str) -> Option<String> {
    encode_segment_bytes(segment.as_bytes())
}

fn encode_segment_bytes(segment: &[u8]) -> Option<String> {
    if matches!(segment, b"." | b"..") {
        return None;
    }
    Some(
        url::form_urlencoded::byte_serialize(segment)
            .map(|part| if part == "+" { "%20" } else { part })
            .collect(),
    )
}

struct CachedToken {
    value: Secret<String>,
    refresh_at: Instant,
    expires_at: Instant,
}
impl CachedToken {
    fn needs_refresh(&self, now: Instant) -> bool {
        now >= self.refresh_at
    }
    fn usable(&self, now: Instant) -> bool {
        now < self.expires_at
    }
}

struct TokenGrant {
    token_url: Url,
    client_id: String,
    client_secret: Secret<String>,
}

type TokenCache = StaleCache<Arc<CachedToken>>;

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
    expires_in: u64,
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: ErrorCode,
}
#[derive(Deserialize)]
struct ErrorCode {
    code: String,
}

#[derive(Clone)]
enum Credentials {
    Static(Secret<String>),
    ClientCredentials {
        grant: Arc<TokenGrant>,
        cache: Arc<TokenCache>,
        clock: Arc<dyn Clock>,
    },
}

/// Calls `/platform/v1` with a static service token or a client-credentials
/// token. Clones share the token cache, so one token request serves
/// concurrent callers.
#[derive(Clone)]
pub struct HubClient {
    product_id: String,
    api_url: Url,
    credentials: Credentials,
    client: GuardedClient,
}
impl HubClient {
    /// The guarded client must capture error bodies through
    /// `EgressOptions::with_error_body_limit`, so hub error codes reach typed errors.
    ///
    /// # Errors
    ///
    /// `ValidationErrors` when `config` is invalid, `product_id` is empty, the client does not
    /// capture error bodies, or the API URL, token URL or client secret is missing or malformed.
    pub fn new(
        config: &HubClientConfig,
        product_id: &str,
        client: GuardedClient,
    ) -> Result<Self, ValidationErrors> {
        Self::with_clock(config, product_id, client, Arc::new(SystemClock))
    }
    /// [`Self::new`] with the clock that drives token expiry and the token
    /// failure cooldown.
    ///
    /// # Errors
    ///
    /// The errors of [`Self::new`].
    pub fn with_clock(
        config: &HubClientConfig,
        product_id: &str,
        client: GuardedClient,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, ValidationErrors> {
        config.validate()?;
        let invalid =
            |field, message| ValidationErrors::new(vec![ValidationError::new(field, message)]);
        if product_id.is_empty() {
            return Err(invalid("platform.product_id", "is required for hub calls"));
        }
        if client.options().error_body_limit().is_none() {
            return Err(invalid(
                "platform_client.egress",
                "must capture error bodies to read hub error codes",
            ));
        }
        let settings_error = || invalid("platform_client", "invalid hub client settings");
        let api_url = base_url(&config.api_url).ok_or_else(settings_error)?;
        let credentials = match config.static_token() {
            Some(token) => Credentials::Static(token.clone()),
            None => Credentials::ClientCredentials {
                grant: Arc::new(TokenGrant {
                    token_url: config
                        .token_url
                        .as_deref()
                        .and_then(base_url)
                        .ok_or_else(settings_error)?,
                    client_id: config
                        .client_id
                        .clone()
                        .unwrap_or_else(|| format!("{CLIENT_ID_PREFIX}{product_id}")),
                    client_secret: config.client_secret.clone().ok_or_else(settings_error)?,
                }),
                cache: Arc::new(StaleCache::with_clock(
                    StalePolicy::new(MAX_TOKEN_LIFETIME, Duration::ZERO, TOKEN_FAILURE_COOLDOWN),
                    Arc::clone(&clock),
                )),
                clock,
            },
        };
        Ok(Self {
            product_id: product_id.into(),
            api_url,
            credentials,
            client,
        })
    }
    /// The calling product. The hub derives wallets and summaries from it.
    pub fn product_id(&self) -> &str {
        &self.product_id
    }
    /// `GET <api_url>/platform/v1/<segments>`. Each segment is percent-encoded.
    ///
    /// # Errors
    ///
    /// The errors of [`Self::send_json`].
    pub async fn get_json<T: DeserializeOwned>(&self, segments: &[&str]) -> Result<T, HubError> {
        self.send_json(HubRequest::get(segments)).await
    }
    /// `GET <api_url>/platform/v1/<segments>?<query>`. Names and values are
    /// form-encoded in the given order. An empty `query` sends no `?`.
    ///
    /// # Errors
    ///
    /// The errors of [`Self::send_json`].
    pub async fn get_json_with_query<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        query: &[(&str, &str)],
    ) -> Result<T, HubError> {
        self.send_json(HubRequest::get_with_query(segments, query))
            .await
    }
    /// Sends `request` with the service token and decodes the JSON response.
    ///
    /// After a 401 with a client-credentials token, the request is repeated
    /// once with a fresh token only when it is replayable: a GET, a
    /// [`HubRequest::idempotent`] POST, or one with an idempotency key.
    ///
    /// # Errors
    ///
    /// `HubError::Unavailable` on transport failure, timeout, 408, 429 or 5xx. `CoolingDown` when the
    /// egress breaker is open, the client is saturated or token requests cool down after a
    /// failure. `Unauthorized` when the token endpoint refuses the credentials or the API returns
    /// 401 and the request is not replayed or fails again. `Rejected` for other statuses.
    /// `InvalidResponse` when a URL cannot be built, the body is too large or does not decode into
    /// `T`, or the token response is malformed.
    pub async fn send_json<T: DeserializeOwned>(&self, request: HubRequest) -> Result<T, HubError> {
        let egress = self.egress_request(&request)?;
        let token = self.token().await?;
        match self.execute(&egress, authorization(&token)?).await {
            Err(EgressError::Status { status, .. })
                if status == StatusCode::UNAUTHORIZED
                    && request.replay() == Replay::Allowed
                    && self.refreshes() =>
            {
                let token = self.refreshed_token(&token).await?;
                decode(self.execute(&egress, authorization(&token)?).await)
            }
            result => decode(result),
        }
    }
    fn egress_request(&self, request: &HubRequest) -> Result<EgressRequest, HubError> {
        let mut url = self.url(&request.segment_refs())?;
        if !request.query().is_empty() {
            url.query_pairs_mut().extend_pairs(request.query());
        }
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT, HeaderValue::from_static(JSON));
        if let Some(key) = request.idempotency_key() {
            headers.insert(IDEMPOTENCY_KEY, key.clone());
        }
        let egress = match request.body() {
            Some(body) => {
                headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(JSON));
                EgressRequest::post(url).with_body(body.to_vec())
            }
            None => EgressRequest::get(url),
        };
        Ok(egress.with_headers(headers))
    }
    async fn execute(
        &self,
        request: &EgressRequest,
        authorization: HeaderValue,
    ) -> Result<EgressResponse, EgressError> {
        let mut headers = request.headers().clone();
        headers.insert(header::AUTHORIZATION, authorization);
        self.client
            .execute(request.clone().with_headers(headers))
            .await
    }
    fn url(&self, segments: &[&str]) -> Result<Url, HubError> {
        let mut url = self.api_url.clone();
        if url.cannot_be_a_base() {
            return Err(HubError::InvalidResponse);
        }
        let mut path = url.path().trim_end_matches('/').to_owned();
        for segment in ["platform", "v1"].iter().chain(segments) {
            path.push('/');
            path.push_str(&encode_segment(segment).ok_or(HubError::Rejected {
                status: 0,
                code: Some(DOT_SEGMENT_CODE.to_owned()),
            })?);
        }
        url.set_path(&path);
        Ok(url)
    }
    #[cfg(test)]
    fn url_with_query(&self, segments: &[&str], query: &[(&str, &str)]) -> Result<Url, HubError> {
        self.egress_request(&HubRequest::get_with_query(segments, query))
            .map(|request| request.url().clone())
    }
    fn refreshes(&self) -> bool {
        matches!(self.credentials, Credentials::ClientCredentials { .. })
    }
    async fn token(&self) -> Result<Secret<String>, HubError> {
        let (grant, cache, clock) = match &self.credentials {
            Credentials::Static(token) => return Ok(token.clone()),
            Credentials::ClientCredentials {
                grant,
                cache,
                clock,
            } => (grant, cache, clock),
        };
        let read = cache
            .get_or_refresh(|| self.fetch_token(grant, clock.as_ref()))
            .await
            .map_err(token_error)?;
        if !read.value.needs_refresh(clock.now()) {
            return Ok(read.value.value.clone());
        }
        let read = cache
            .force_refresh(|| self.fetch_token(grant, clock.as_ref()))
            .await
            .map_err(token_error)?;
        usable_token(&read, clock.now())
    }
    /// A new token after the API refused `rejected`. Concurrent callers share
    /// one token request. Within the cooldown after the last token request the
    /// cached token is `rejected` itself, so the call is not repeated.
    async fn refreshed_token(&self, rejected: &Secret<String>) -> Result<Secret<String>, HubError> {
        let Credentials::ClientCredentials {
            grant,
            cache,
            clock,
        } = &self.credentials
        else {
            return Err(HubError::Unauthorized);
        };
        let read = cache
            .force_refresh(|| self.fetch_token(grant, clock.as_ref()))
            .await
            .map_err(token_error)?;
        let token = usable_token(&read, clock.now())?;
        if token.expose() == rejected.expose() {
            return Err(HubError::Unauthorized);
        }
        Ok(token)
    }
    async fn fetch_token(
        &self,
        grant: &TokenGrant,
        clock: &dyn Clock,
    ) -> Result<Arc<CachedToken>, HubError> {
        let requested_at = clock.now();
        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "client_credentials")
            .append_pair("client_id", &grant.client_id)
            .append_pair("client_secret", grant.client_secret.expose())
            .finish();
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(FORM));
        headers.insert(header::ACCEPT, HeaderValue::from_static(JSON));
        let response = self
            .client
            .execute(
                EgressRequest::post(grant.token_url.clone())
                    .with_headers(headers)
                    .with_body(body),
            )
            .await
            .map_err(|error| match classify(&error) {
                HubError::Rejected { .. } => HubError::Unauthorized,
                other => other,
            })?;
        let response: TokenResponse =
            serde_json::from_slice(response.body()).map_err(|_| HubError::InvalidResponse)?;
        if response.access_token.is_empty()
            || !response.token_type.eq_ignore_ascii_case("bearer")
            || response.expires_in == 0
        {
            return Err(HubError::InvalidResponse);
        }
        let lifetime = Duration::from_secs(response.expires_in).min(MAX_TOKEN_LIFETIME);
        let expires_at = requested_at
            .checked_add(lifetime)
            .ok_or(HubError::InvalidResponse)?;
        Ok(Arc::new(CachedToken {
            value: Secret::new(response.access_token),
            refresh_at: expires_at
                .checked_sub(TOKEN_REFRESH_MARGIN.min(lifetime / 2))
                .unwrap_or(requested_at),
            expires_at,
        }))
    }
}

/// A token that is fresh, or still valid while a refresh failed or cools down.
fn usable_token(
    read: &CacheRead<Arc<CachedToken>>,
    now: Instant,
) -> Result<Secret<String>, HubError> {
    let fetched = read.freshness == Freshness::Fetched && !read.refresh_failed;
    if fetched || read.value.usable(now) {
        return Ok(read.value.value.clone());
    }
    Err(HubError::Unavailable)
}

fn authorization(token: &Secret<String>) -> Result<HeaderValue, HubError> {
    let mut value = HeaderValue::from_str(&format!("Bearer {}", token.expose()))
        .map_err(|_| HubError::Unauthorized)?;
    value.set_sensitive(true);
    Ok(value)
}

fn token_error(error: StaleCacheError<HubError>) -> HubError {
    match error {
        StaleCacheError::Fetch(error) => error,
        StaleCacheError::CoolingDown { retry_after } => HubError::CoolingDown { retry_after },
    }
}

fn decode<T: DeserializeOwned>(result: Result<EgressResponse, EgressError>) -> Result<T, HubError> {
    let response = result.map_err(|error| classify(&error))?;
    serde_json::from_slice(response.body()).map_err(|_| HubError::InvalidResponse)
}

fn classify(error: &EgressError) -> HubError {
    match error {
        EgressError::Status { status, body, .. } => {
            if *status == StatusCode::UNAUTHORIZED {
                HubError::Unauthorized
            } else if status.is_server_error()
                || *status == StatusCode::TOO_MANY_REQUESTS
                || *status == StatusCode::REQUEST_TIMEOUT
            {
                HubError::Unavailable
            } else {
                HubError::Rejected {
                    status: status.as_u16(),
                    code: body.as_ref().and_then(|body| {
                        serde_json::from_slice::<ErrorEnvelope>(body.bytes())
                            .ok()
                            .map(|envelope| envelope.error.code)
                    }),
                }
            }
        }
        EgressError::ResponseTooLarge { .. } => HubError::InvalidResponse,
        EgressError::CircuitOpen { .. } | EgressError::Saturated => HubError::CoolingDown {
            retry_after: error
                .retry_class()
                .retry_after()
                .unwrap_or(TOKEN_FAILURE_COOLDOWN),
        },
        _ => HubError::Unavailable,
    }
}

fn base_url(value: &str) -> Option<Url> {
    Url::parse(value).ok().filter(|url| {
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client() -> HubClient {
        let config = HubClientConfig {
            api_url: "https://hub.example/api/".into(),
            token: None,
            token_url: Some("https://hub.example/token".into()),
            client_id: None,
            client_secret: Some(Secret::new("secret".into())),
        };
        let guarded = GuardedClient::new(
            "hub",
            baukit_egress::EgressOptions::default()
                .with_error_body_limit(1024)
                .expect("limit"),
        )
        .expect("client");
        HubClient::new(&config, "laufbefund", guarded).expect("hub client")
    }

    fn query_of(query: &[(&str, &str)]) -> Option<String> {
        client()
            .url_with_query(&["organizations", "a/b c", "members"], query)
            .expect("url")
            .query()
            .map(str::to_owned)
    }

    #[test]
    fn open_circuits_and_saturation_cool_down_with_the_breaker_delay() {
        let retry_after = Duration::from_secs(17);
        assert_eq!(
            classify(&EgressError::CircuitOpen { retry_after }),
            HubError::CoolingDown { retry_after }
        );
        assert!(matches!(
            classify(&EgressError::Saturated),
            HubError::CoolingDown { retry_after } if retry_after > Duration::ZERO
        ));
    }

    #[test]
    fn query_values_are_form_encoded_in_order() {
        assert_eq!(
            query_of(&[("product", "laufbefund"), ("cursor", "a+b/c=d&e?f#g%h"),]).as_deref(),
            Some("product=laufbefund&cursor=a%2Bb%2Fc%3Dd%26e%3Ff%23g%25h")
        );
    }

    #[test]
    fn query_encodes_spaces_non_ascii_empty_values_and_names() {
        assert_eq!(
            query_of(&[("q", "Schlüssel bund"), ("empty", ""), ("a&b=c", "1")]).as_deref(),
            Some("q=Schl%C3%BCssel+bund&empty=&a%26b%3Dc=1")
        );
        assert_eq!(
            query_of(&[("product", "x"), ("product", "y")]).as_deref(),
            Some("product=x&product=y")
        );
    }

    #[test]
    fn an_empty_query_keeps_the_get_json_url() {
        let hub = client();
        let segments = ["organizations", "a/b c", "members"];
        let plain = hub.url(&segments).expect("url");
        assert_eq!(hub.url_with_query(&segments, &[]).expect("url"), plain);
        assert_eq!(
            plain.as_str(),
            "https://hub.example/api/platform/v1/organizations/a%2Fb%20c/members"
        );
        assert_eq!(
            hub.url(&["accounts", "suite-dev|alice", "organizations"])
                .expect("url")
                .path(),
            "/api/platform/v1/accounts/suite-dev%7Calice/organizations"
        );
    }

    #[test]
    fn dot_segments_stay_inside_the_platform_api() {
        let hub = client();
        for subject in ["..", "."] {
            assert!(hub.url(&["accounts", subject, "organizations"]).is_err());
        }
        let encoded = hub
            .url(&["accounts", "%2E%2E", "organizations"])
            .expect("url");
        assert_eq!(
            encoded.path(),
            "/api/platform/v1/accounts/%252E%252E/organizations"
        );
    }
}

/// The K-21 property, shared by the Kani proofs and the proptest.
#[cfg(test)]
fn segment_stays_in_its_slot(segment: &[u8]) -> bool {
    if encode_segment_bytes(segment).is_none() {
        return true;
    }
    let mut dots = 0_usize;
    let mut only_dots = true;
    let mut escape = [0_u8; 3];
    let mut escape_len = 0;
    for byte in url::form_urlencoded::byte_serialize(segment)
        .map(|part| if part == "+" { "%20" } else { part })
        .flat_map(str::bytes)
    {
        if matches!(byte, b'/' | b'?' | b'#' | b'\\') {
            return false;
        }
        if byte == b'%' || escape_len > 0 {
            escape[escape_len] = byte;
            escape_len += 1;
            if escape_len == escape.len() {
                if escape.eq_ignore_ascii_case(b"%2e") {
                    dots += 1;
                } else {
                    only_dots = false;
                }
                escape_len = 0;
            }
        } else if byte == b'.' {
            dots += 1;
        } else {
            only_dots = false;
        }
    }
    !(only_dots && matches!(dots, 1 | 2))
}

#[cfg(test)]
mod segment_tests {
    use super::segment_stays_in_its_slot;
    use proptest::prelude::*;

    proptest! {
        /// K-21 beyond the Kani bound: up to 16 arbitrary chars.
        #[test]
        fn any_segment_stays_in_its_slot(segment in "\\PC{0,16}") {
            prop_assert!(segment_stays_in_its_slot(segment.as_bytes()));
        }

        #[test]
        fn every_byte_encodes_inside_its_slot(byte in any::<u8>()) {
            prop_assert!(super::encoded_byte_stays_in_its_slot(byte));
        }

        #[test]
        fn dot_like_segments_stay_in_their_slot(segment in "[.%2eE]{0,6}") {
            prop_assert!(segment_stays_in_its_slot(segment.as_bytes()));
        }
    }
}

/// Encodes one byte the way `encode_segment` does. `byte_serialize` maps each byte on its own,
/// so a segment stays in its slot when every byte does and the only dot-only inputs, `.` and
/// `..`, are refused.
#[cfg(any(kani, test))]
fn encoded_byte_stays_in_its_slot(byte: u8) -> bool {
    const ENCODED_DOT: &[u8] = b"%2e";
    let input = [byte];
    let Some(part) = url::form_urlencoded::byte_serialize(&input).next() else {
        return false;
    };
    let part = if part == "+" { "%20" } else { part }.as_bytes();
    let forbidden = part
        .iter()
        .any(|encoded| matches!(encoded, b'/' | b'?' | b'#' | b'\\'));
    let dot = part == b"." || part.eq_ignore_ascii_case(ENCODED_DOT);
    !forbidden && dot == (byte == b'.')
}

#[cfg(kani)]
mod proofs {
    use super::{encode_segment_bytes, encoded_byte_stays_in_its_slot};

    /// K-21 over every byte string of up to 3 bytes, a superset of every string of up to 3
    /// bytes. The encoder maps bytes one by one, so the proof checks each byte's encoding and
    /// that the dot-only segments are refused. Bounds: 3 bytes, unwind 12.
    #[kani::proof]
    #[kani::unwind(12)]
    fn an_encoded_segment_cannot_leave_its_slot() {
        let bytes: [u8; 3] = kani::any();
        for byte in bytes {
            assert!(encoded_byte_stays_in_its_slot(byte));
        }
        assert!(encode_segment_bytes(b".").is_none());
        assert!(encode_segment_bytes(b"..").is_none());
    }

    /// K-21 for any single `char`, 1 to 4 bytes. Bounds: one char, unwind 12.
    #[kani::proof]
    #[kani::unwind(12)]
    fn any_single_char_segment_stays_in_its_slot() {
        let c: char = kani::any();
        let mut buffer = [0_u8; 4];
        for byte in c.encode_utf8(&mut buffer).bytes() {
            assert!(encoded_byte_stays_in_its_slot(byte));
        }
    }
}
