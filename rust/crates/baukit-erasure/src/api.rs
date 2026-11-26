use crate::{ErasureFuture, IdentityAccountDeleter, IdentityDeletionError};
use baukit_config::Secret;
use baukit_egress::{EgressRequest, GuardedClient};
use url::Url;

use crate::egress::{
    MissingUserDialect, bearer, deletion_outcome, endpoint_client, is_loopback, status, user_url,
};
use crate::kernel::deletion_status_outcome;

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

const PROBE_QUERY: &str = "limit=1";

#[derive(Clone)]
struct ApiDeleter {
    client: GuardedClient,
    base: Url,
    key: Secret<String>,
    parts: &'static [&'static str],
    dialect: MissingUserDialect,
}

impl ApiDeleter {
    fn new(
        config: ApiDeletionConfig,
        parts: &'static [&'static str],
        dialect: MissingUserDialect,
    ) -> Result<Self, IdentityDeletionError> {
        let base = Url::parse(&config.base_url).map_err(|_| IdentityDeletionError::Permanent)?;
        let local_http = base.scheme() == "http" && is_loopback(&base) && config.allow_local_http;
        if !(base.scheme() == "https" || local_http)
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || config.api_key.expose().is_empty()
        {
            return Err(IdentityDeletionError::Permanent);
        }
        let client = endpoint_client(&base)?;
        Ok(Self {
            client,
            base,
            key: config.api_key,
            parts,
            dialect,
        })
    }

    async fn delete(&self, subject: &str) -> Result<(), IdentityDeletionError> {
        let url = user_url(&self.base, self.parts, subject)?;
        let request = EgressRequest::new(axum::http::Method::DELETE, url)
            .with_headers(bearer(self.key.expose())?);
        deletion_outcome(self.dialect, &status(&self.client, request).await?)
    }

    async fn probe(&self) -> Result<(), IdentityDeletionError> {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .map_err(|_| IdentityDeletionError::Permanent)?
            .pop_if_empty()
            .extend(self.parts);
        url.set_query(Some(PROBE_QUERY));
        let request = EgressRequest::new(axum::http::Method::GET, url)
            .with_headers(bearer(self.key.expose())?);
        let (status, _) = status(&self.client, request).await?;
        deletion_status_outcome(status.as_u16(), false)
    }
}

/// Clerk Backend API user deletion. WorkerRunner retries temporary failures.
#[derive(Clone)]
pub struct ClerkAccountDeleter(ApiDeleter);

impl ClerkAccountDeleter {
    /// Uses a Clerk API root, normally `https://api.clerk.com/v1`.
    ///
    /// # Errors
    ///
    /// [`IdentityDeletionError::Permanent`] for an unparsable or non-HTTPS URL (HTTP only on
    /// loopback with `allow_local_http`), a URL with credentials, query or fragment, an empty
    /// API key, or when the egress client cannot be built.
    pub fn new(config: ApiDeletionConfig) -> Result<Self, IdentityDeletionError> {
        ApiDeleter::new(config, &["users"], MissingUserDialect::Clerk).map(Self)
    }
}

impl IdentityAccountDeleter for ClerkAccountDeleter {
    fn delete_account<'a>(
        &'a self,
        subject: &'a str,
    ) -> ErasureFuture<'a, Result<(), IdentityDeletionError>> {
        Box::pin(self.0.delete(subject))
    }
    fn probe(&self) -> ErasureFuture<'_, Result<(), IdentityDeletionError>> {
        Box::pin(self.0.probe())
    }
}

/// WorkOS User Management deletion. WorkerRunner retries temporary failures.
#[derive(Clone)]
pub struct WorkOsAccountDeleter(ApiDeleter);

impl WorkOsAccountDeleter {
    /// Uses a WorkOS API root, normally `https://api.workos.com`.
    ///
    /// # Errors
    ///
    /// [`IdentityDeletionError::Permanent`] for an unparsable or non-HTTPS URL (HTTP only on
    /// loopback with `allow_local_http`), a URL with credentials, query or fragment, an empty
    /// API key, or when the egress client cannot be built.
    pub fn new(config: ApiDeletionConfig) -> Result<Self, IdentityDeletionError> {
        ApiDeleter::new(
            config,
            &["user_management", "users"],
            MissingUserDialect::WorkOs,
        )
        .map(Self)
    }
}

impl IdentityAccountDeleter for WorkOsAccountDeleter {
    fn delete_account<'a>(
        &'a self,
        subject: &'a str,
    ) -> ErasureFuture<'a, Result<(), IdentityDeletionError>> {
        Box::pin(self.0.delete(subject))
    }
    fn probe(&self) -> ErasureFuture<'_, Result<(), IdentityDeletionError>> {
        Box::pin(self.0.probe())
    }
}
