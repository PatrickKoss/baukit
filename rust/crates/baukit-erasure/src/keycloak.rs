use crate::egress::{bearer, endpoint_client, status, success};
use crate::{ErasureFuture, IdentityAccountDeleter, IdentityDeletionError};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use baukit_config::Secret;
use baukit_egress::{EgressRequest, GuardedClient};
use serde::Deserialize;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use url::Url;

/// Confidential service-account configuration.
#[derive(Clone, Debug)]
pub struct KeycloakDeletionConfig {
    /// Keycloak base URL, including any deployment prefix.
    pub base_url: String,
    /// Product realm.
    pub realm: String,
    /// Confidential backend client with manage-users.
    pub client_id: String,
    /// Client credential loaded from configuration.
    pub client_secret: Secret<String>,
    /// Explicit development-only permission to use HTTP.
    pub allow_local_http: bool,
}

/// Dedicated Keycloak admin client that reaches only the configured base URL's
/// scheme, host and port, with no proxy or redirects.
#[derive(Clone)]
pub struct KeycloakAccountDeleter {
    client: GuardedClient,
    base: Url,
    config: KeycloakDeletionConfig,
    token: Arc<Mutex<Option<CachedToken>>>,
}
struct CachedToken {
    value: Secret<String>,
    expiry: Instant,
}
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
}

impl KeycloakAccountDeleter {
    /// Validates configuration and constructs the private-network client.
    pub fn new(config: KeycloakDeletionConfig) -> Result<Self, IdentityDeletionError> {
        let base = Url::parse(&config.base_url).map_err(|_| IdentityDeletionError::Permanent)?;
        if !(base.scheme() == "https" || (base.scheme() == "http" && config.allow_local_http))
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || config.realm.is_empty()
            || config.client_id.is_empty()
            || config.client_secret.expose().is_empty()
        {
            return Err(IdentityDeletionError::Permanent);
        }
        let client = endpoint_client(&base)?;
        Ok(Self {
            client,
            base,
            config,
            token: Arc::new(Mutex::new(None)),
        })
    }
    fn url(&self, parts: &[&str]) -> Result<Url, IdentityDeletionError> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| IdentityDeletionError::Permanent)?
            .pop_if_empty()
            .extend(parts);
        Ok(url)
    }
    async fn token(&self) -> Result<String, IdentityDeletionError> {
        let mut cached = self.token.lock().await;
        if let Some(token) = &*cached
            && Instant::now() < token.expiry
        {
            return Ok(token.value.expose().clone());
        }
        let form = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("grant_type", "client_credentials")
            .append_pair("client_id", &self.config.client_id)
            .append_pair("client_secret", self.config.client_secret.expose())
            .finish();
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        let request = EgressRequest::post(self.url(&[
            "realms",
            &self.config.realm,
            "protocol",
            "openid-connect",
            "token",
        ])?)
        .with_headers(headers)
        .with_body(form);
        let response = success(&self.client, request).await?;
        let token: TokenResponse = serde_json::from_slice(response.body())
            .map_err(|_| IdentityDeletionError::Retryable)?;
        if token.access_token.is_empty() {
            return Err(IdentityDeletionError::Retryable);
        }
        let expiry = Instant::now() + Duration::from_secs(token.expires_in.saturating_sub(30));
        *cached = Some(CachedToken {
            value: Secret::new(token.access_token.clone()),
            expiry,
        });
        Ok(token.access_token)
    }
}
impl IdentityAccountDeleter for KeycloakAccountDeleter {
    fn delete_account<'a>(
        &'a self,
        subject: &'a str,
    ) -> ErasureFuture<'a, Result<(), IdentityDeletionError>> {
        Box::pin(async move {
            if subject.is_empty() {
                return Err(IdentityDeletionError::Permanent);
            }
            let url = self.url(&["admin", "realms", &self.config.realm, "users", subject])?;
            let request =
                EgressRequest::new(Method::DELETE, url).with_headers(bearer(&self.token().await?)?);
            let status = status(&self.client, request).await?;
            if status.is_success() || status == StatusCode::NOT_FOUND {
                return Ok(());
            }
            if status == StatusCode::UNAUTHORIZED {
                *self.token.lock().await = None;
                return Err(IdentityDeletionError::Retryable);
            }
            if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
                Err(IdentityDeletionError::Retryable)
            } else {
                Err(IdentityDeletionError::Permanent)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn requires_https_and_redacts_secret() {
        let mut config = KeycloakDeletionConfig {
            base_url: "http://localhost".into(),
            realm: "test".into(),
            client_id: "backend".into(),
            client_secret: Secret::new("secret".into()),
            allow_local_http: false,
        };
        assert!(KeycloakAccountDeleter::new(config.clone()).is_err());
        config.allow_local_http = true;
        assert!(KeycloakAccountDeleter::new(config.clone()).is_ok());
        assert!(!format!("{config:?}").contains("\"secret\""));
        config.base_url = "https://user:pass@example.com".into();
        assert!(KeycloakAccountDeleter::new(config).is_err());
    }
}
