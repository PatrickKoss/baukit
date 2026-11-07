use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use baukit_config::{Secret, Validate, ValidationError, ValidationErrors};
use baukit_egress::{EgressError, EgressRequest, GuardedClient};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{sync::Arc, time::Duration};
use tokio::{sync::Mutex, time::Instant};
use url::Url;

/// Tokens are replaced this long before `expires_in` elapses, or at half their
/// lifetime when that is shorter.
pub const TOKEN_REFRESH_MARGIN: Duration = Duration::from_secs(30);
const CLIENT_ID_PREFIX: &str = "platform-";
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
fn encode_segment(segment: &str) -> String {
    url::form_urlencoded::byte_serialize(segment.as_bytes())
        .map(|part| if part == "+" { "%20" } else { part })
        .collect()
}

struct CachedToken {
    value: Secret<String>,
    refresh_at: Instant,
}

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
    Static(Arc<CachedToken>),
    ClientCredentials {
        token_url: Url,
        client_id: String,
        client_secret: Secret<String>,
        cache: Arc<Mutex<Option<Arc<CachedToken>>>>,
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
            Some(token) => Credentials::Static(Arc::new(CachedToken {
                value: token.clone(),
                refresh_at: Instant::now(),
            })),
            None => Credentials::ClientCredentials {
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
                cache: Arc::default(),
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
    /// `HubError::Unavailable` on transport failure, timeout, 408, 429 or 5xx. `Unauthorized` when
    /// the token endpoint refuses the credentials or the API returns 401 after any refresh.
    /// `Rejected` for other statuses. `InvalidResponse` when the body is too large or does not
    /// decode into `T`, or the token response is malformed.
    pub async fn get_json<T: DeserializeOwned>(&self, segments: &[&str]) -> Result<T, HubError> {
        self.send(EgressRequest::get(self.url(segments)?)).await
    }
    /// `GET <api_url>/platform/v1/<segments>?<query>`. Names and values are
    /// form-encoded in the given order. An empty `query` sends no `?`.
    ///
    /// # Errors
    ///
    /// The errors of [`Self::get_json`].
    pub async fn get_json_with_query<T: DeserializeOwned>(
        &self,
        segments: &[&str],
        query: &[(&str, &str)],
    ) -> Result<T, HubError> {
        self.send(EgressRequest::get(self.url_with_query(segments, query)?))
            .await
    }
    /// `POST <api_url>/platform/v1/<segments>` with a JSON body. A 401 retry
    /// repeats the request, so use it only for idempotent routes.
    ///
    /// # Errors
    ///
    /// The errors of [`Self::get_json`]. `HubError::InvalidResponse` also covers a body that does
    /// not serialize.
    pub async fn post_json<B: Serialize + Sync, T: DeserializeOwned>(
        &self,
        segments: &[&str],
        body: &B,
    ) -> Result<T, HubError> {
        let body = serde_json::to_vec(body).map_err(|_| HubError::InvalidResponse)?;
        self.send(EgressRequest::post(self.url(segments)?).with_body(body))
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
            path.push_str(&encode_segment(segment));
        }
        url.set_path(&path);
        Ok(url)
    }
    fn url_with_query(&self, segments: &[&str], query: &[(&str, &str)]) -> Result<Url, HubError> {
        let mut url = self.url(segments)?;
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        Ok(url)
    }
    async fn send<T: DeserializeOwned>(&self, request: EgressRequest) -> Result<T, HubError> {
        let has_body = !request.body().is_empty();
        let mut refreshed = false;
        loop {
            let token = self.token().await?;
            let mut headers = HeaderMap::new();
            headers.insert(header::ACCEPT, HeaderValue::from_static(JSON));
            if has_body {
                headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(JSON));
            }
            let mut authorization =
                HeaderValue::from_str(&format!("Bearer {}", token.value.expose()))
                    .map_err(|_| HubError::Unauthorized)?;
            authorization.set_sensitive(true);
            headers.insert(header::AUTHORIZATION, authorization);
            match self
                .client
                .execute(request.clone().with_headers(headers))
                .await
            {
                Ok(response) => {
                    return serde_json::from_slice(response.body())
                        .map_err(|_| HubError::InvalidResponse);
                }
                Err(EgressError::Status { status, .. })
                    if status == StatusCode::UNAUTHORIZED && !refreshed && self.refreshes() =>
                {
                    refreshed = true;
                    self.invalidate(&token).await;
                }
                Err(error) => return Err(classify(&error)),
            }
        }
    }
    fn refreshes(&self) -> bool {
        matches!(self.credentials, Credentials::ClientCredentials { .. })
    }
    async fn invalidate(&self, used: &Arc<CachedToken>) {
        let Credentials::ClientCredentials { cache, .. } = &self.credentials else {
            return;
        };
        let mut cached = cache.lock().await;
        if cached
            .as_ref()
            .is_some_and(|token| Arc::ptr_eq(token, used))
        {
            *cached = None;
        }
    }
    async fn token(&self) -> Result<Arc<CachedToken>, HubError> {
        let (token_url, client_id, client_secret, cache) = match &self.credentials {
            Credentials::Static(token) => return Ok(token.clone()),
            Credentials::ClientCredentials {
                token_url,
                client_id,
                client_secret,
                cache,
            } => (token_url, client_id, client_secret, cache),
        };
        let mut cached = cache.lock().await;
        if let Some(token) = cached
            .as_ref()
            .filter(|token| Instant::now() < token.refresh_at)
        {
            return Ok(token.clone());
        }
        let requested_at = Instant::now();
        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "client_credentials")
            .append_pair("client_id", client_id)
            .append_pair("client_secret", client_secret.expose())
            .finish();
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(FORM));
        headers.insert(header::ACCEPT, HeaderValue::from_static(JSON));
        let response = self
            .client
            .execute(
                EgressRequest::post(token_url.clone())
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
        let lifetime = Duration::from_secs(response.expires_in);
        let token = Arc::new(CachedToken {
            value: Secret::new(response.access_token),
            refresh_at: requested_at + lifetime - TOKEN_REFRESH_MARGIN.min(lifetime / 2),
        });
        *cached = Some(token.clone());
        Ok(token)
    }
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
            baukit_egress::EgressOptions::default()
                .with_error_body_limit(1024)
                .expect("limit"),
        )
        .expect("client");
        HubClient::new(&config, "runtime_analyzer", guarded).expect("hub client")
    }

    fn query_of(query: &[(&str, &str)]) -> Option<String> {
        client()
            .url_with_query(&["organizations", "a/b c", "members"], query)
            .expect("url")
            .query()
            .map(str::to_owned)
    }

    #[test]
    fn query_values_are_form_encoded_in_order() {
        assert_eq!(
            query_of(&[
                ("product", "runtime_analyzer"),
                ("cursor", "a+b/c=d&e?f#g%h"),
            ])
            .as_deref(),
            Some("product=runtime_analyzer&cursor=a%2Bb%2Fc%3Dd%26e%3Ff%23g%25h")
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
}
