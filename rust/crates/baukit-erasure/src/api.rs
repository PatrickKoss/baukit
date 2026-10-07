use crate::{ErasureFuture, IdentityAccountDeleter, IdentityDeletionError};
use baukit_config::Secret;
use reqwest::{Client, StatusCode};
use std::time::Duration;
use url::Url;

/// API credentials for an identity provider. The key never appears in errors.
#[derive(Clone, Debug)]
pub struct ApiDeletionConfig {
    /// Provider API root, including its version prefix where required.
    pub base_url: String,
    /// Server-side API credential loaded through the secret configuration.
    pub api_key: Secret<String>,
    /// Explicit permission for loopback HTTP in tests and local development.
    pub allow_local_http: bool,
}

#[derive(Clone)]
struct ApiDeleter {
    client: Client,
    base: Url,
    key: Secret<String>,
}

impl ApiDeleter {
    fn new(config: ApiDeletionConfig) -> Result<Self, IdentityDeletionError> {
        let base = Url::parse(&config.base_url).map_err(|_| IdentityDeletionError::Permanent)?;
        let local = matches!(base.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
        if !(base.scheme() == "https"
            || base.scheme() == "http" && local && config.allow_local_http)
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || config.api_key.expose().is_empty()
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
            key: config.api_key,
        })
    }

    async fn delete(&self, parts: &[&str], subject: &str) -> Result<(), IdentityDeletionError> {
        if subject.is_empty() || matches!(subject, "." | "..") {
            return Err(IdentityDeletionError::Permanent);
        }
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| IdentityDeletionError::Permanent)?
            .pop_if_empty()
            .extend(parts)
            .push(subject);
        let status = self
            .client
            .delete(url)
            .bearer_auth(self.key.expose())
            .send()
            .await
            .map_err(|_| IdentityDeletionError::Retryable)?
            .status();
        if status.is_success() || status == StatusCode::NOT_FOUND {
            return Ok(());
        }
        if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
            Err(IdentityDeletionError::Retryable)
        } else {
            Err(IdentityDeletionError::Permanent)
        }
    }
}

/// Clerk Backend API user deletion. WorkerRunner retries temporary failures.
#[derive(Clone)]
pub struct ClerkAccountDeleter(ApiDeleter);

impl ClerkAccountDeleter {
    /// Uses a Clerk API root, normally `https://api.clerk.com/v1`.
    pub fn new(config: ApiDeletionConfig) -> Result<Self, IdentityDeletionError> {
        ApiDeleter::new(config).map(Self)
    }
}

impl IdentityAccountDeleter for ClerkAccountDeleter {
    fn delete_account<'a>(
        &'a self,
        subject: &'a str,
    ) -> ErasureFuture<'a, Result<(), IdentityDeletionError>> {
        Box::pin(self.0.delete(&["users"], subject))
    }
}

/// WorkOS User Management deletion. WorkerRunner retries temporary failures.
#[derive(Clone)]
pub struct WorkOsAccountDeleter(ApiDeleter);

impl WorkOsAccountDeleter {
    /// Uses a WorkOS API root, normally `https://api.workos.com`.
    pub fn new(config: ApiDeletionConfig) -> Result<Self, IdentityDeletionError> {
        ApiDeleter::new(config).map(Self)
    }
}

impl IdentityAccountDeleter for WorkOsAccountDeleter {
    fn delete_account<'a>(
        &'a self,
        subject: &'a str,
    ) -> ErasureFuture<'a, Result<(), IdentityDeletionError>> {
        Box::pin(self.0.delete(&["user_management", "users"], subject))
    }
}
