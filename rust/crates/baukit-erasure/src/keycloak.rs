use crate::{ErasureFuture, IdentityAccountDeleter, IdentityDeletionError};
use baukit_config::Secret;
use reqwest::{Client, StatusCode};
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

/// Dedicated Keycloak admin client, with no proxy or redirects.
#[derive(Clone)]
pub struct KeycloakAccountDeleter {
    client: Client,
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
        let client = Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(2))
            .timeout(Duration::from_secs(5))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| IdentityDeletionError::Permanent)?;
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
        let token: TokenResponse = self
            .client
            .post(self.url(&[
                "realms",
                &self.config.realm,
                "protocol",
                "openid-connect",
                "token",
            ])?)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", self.config.client_id.as_str()),
                ("client_secret", self.config.client_secret.expose().as_str()),
            ])
            .send()
            .await
            .map_err(|_| IdentityDeletionError::Retryable)?
            .error_for_status()
            .map_err(|_| IdentityDeletionError::Retryable)?
            .json()
            .await
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
            let response = self
                .client
                .delete(self.url(&["admin", "realms", &self.config.realm, "users", subject])?)
                .bearer_auth(self.token().await?)
                .send()
                .await
                .map_err(|_| IdentityDeletionError::Retryable)?;
            let status = response.status();
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
