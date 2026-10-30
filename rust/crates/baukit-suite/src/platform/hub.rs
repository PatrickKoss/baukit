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
/// `PlatformConfig`, for example as `platform_client`.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct HubClientConfig {
    /// Hub API base URL. Requests go to `<api_url>/platform/v1/...`.
    pub api_url: String,
    /// The hub realm's OAuth token endpoint.
    pub token_url: String,
    /// Defaults to `platform-<product_id>`.
    #[serde(deserialize_with = "crate::config::empty_string_as_none")]
    pub client_id: Option<String>,
    pub client_secret: Option<Secret<String>>,
}
impl Validate for HubClientConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        let mut errors = Vec::new();
        for (field, value) in [
            ("platform_client.api_url", &self.api_url),
            ("platform_client.token_url", &self.token_url),
        ] {
            if base_url(value).is_none() {
                errors.push(ValidationError::new(
                    field,
                    "must be an HTTP(S) URL without credentials, query or fragment",
                ));
            }
        }
        if self
            .client_id
            .as_deref()
            .is_some_and(|id| id.trim() != id || id.chars().any(char::is_control) || id.is_empty())
        {
            errors.push(ValidationError::new(
                "platform_client.client_id",
                "must not contain whitespace padding or control characters",
            ));
        }
        if !self
            .client_secret
            .as_ref()
            .is_some_and(|secret| !secret.expose().is_empty())
        {
            errors.push(ValidationError::new(
                "platform_client.client_secret",
                "is required",
            ));
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(ValidationErrors::new(errors))
        }
    }
}

/// A hub call that did not return a usable response. No variant carries a URL,
/// token or secret.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum HubError {
    /// Transport failure, timeout, rate limit, or a 408 or 5xx response.
    #[error("hub_unavailable")]
    Unavailable,
    /// The token endpoint refused the credentials, or the API returned 401
    /// again with a freshly issued token.
    #[error("hub_unauthorized")]
    Unauthorized,
    /// A 2xx response whose body does not match the contract.
    #[error("hub_response_invalid")]
    InvalidResponse,
    /// Any other status, with the Baukit error envelope code when present.
    #[error("hub_rejected")]
    Rejected { status: u16, code: Option<String> },
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

/// Calls `/platform/v1` with a client-credentials token. Clones share the
/// token cache, so one token request serves concurrent callers.
#[derive(Clone)]
pub struct HubClient {
    product_id: String,
    api_url: Url,
    token_url: Url,
    client_id: String,
    client_secret: Secret<String>,
    client: GuardedClient,
    token: Arc<Mutex<Option<Arc<CachedToken>>>>,
}
impl HubClient {
    /// The guarded client must capture error bodies through
    /// `EgressOptions::with_error_body_limit`, so hub error codes reach typed errors.
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
        let (Some(api_url), Some(token_url), Some(client_secret)) = (
            base_url(&config.api_url),
            Url::parse(&config.token_url).ok(),
            config.client_secret.clone(),
        ) else {
            return Err(invalid("platform_client", "invalid hub client settings"));
        };
        Ok(Self {
            product_id: product_id.into(),
            api_url,
            token_url,
            client_id: config
                .client_id
                .clone()
                .unwrap_or_else(|| format!("{CLIENT_ID_PREFIX}{product_id}")),
            client_secret,
            client,
            token: Arc::default(),
        })
    }
    /// The calling product. The hub derives wallets and summaries from it.
    pub fn product_id(&self) -> &str {
        &self.product_id
    }
    /// `GET <api_url>/platform/v1/<segments>`. Each segment is percent-encoded.
    pub async fn get_json<T: DeserializeOwned>(&self, segments: &[&str]) -> Result<T, HubError> {
        self.send(EgressRequest::get(self.url(segments)?)).await
    }
    /// `POST <api_url>/platform/v1/<segments>` with a JSON body. A 401 retry
    /// repeats the request, so use it only for idempotent routes.
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
        url.path_segments_mut()
            .map_err(|_| HubError::InvalidResponse)?
            .pop_if_empty()
            .extend(["platform", "v1"])
            .extend(segments);
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
                    if status == StatusCode::UNAUTHORIZED && !refreshed =>
                {
                    refreshed = true;
                    self.invalidate(&token).await;
                }
                Err(error) => return Err(classify(&error)),
            }
        }
    }
    async fn invalidate(&self, used: &Arc<CachedToken>) {
        let mut cached = self.token.lock().await;
        if cached
            .as_ref()
            .is_some_and(|token| Arc::ptr_eq(token, used))
        {
            *cached = None;
        }
    }
    async fn token(&self) -> Result<Arc<CachedToken>, HubError> {
        let mut cached = self.token.lock().await;
        if let Some(token) = cached
            .as_ref()
            .filter(|token| Instant::now() < token.refresh_at)
        {
            return Ok(token.clone());
        }
        let requested_at = Instant::now();
        let body = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "client_credentials")
            .append_pair("client_id", &self.client_id)
            .append_pair("client_secret", self.client_secret.expose())
            .finish();
        let mut headers = HeaderMap::new();
        headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(FORM));
        headers.insert(header::ACCEPT, HeaderValue::from_static(JSON));
        let response = self
            .client
            .execute(
                EgressRequest::post(self.token_url.clone())
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
