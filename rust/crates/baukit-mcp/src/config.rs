use std::time::Duration;

use baukit_auth::{OidcConfig, OidcConfigError, PrincipalClaimMapping};
use baukit_ratelimit::{Quota, QuotaError};
use http::uri::Authority;
use serde::{Deserialize, Serialize};
use url::Url;

const DEFAULT_BODY_LIMIT: usize = 32 * 1024;

/// Product configuration for one MCP resource at `/mcp`.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct McpConfig {
    pub enabled: bool,
    pub resource_url: String,
    pub issuer: String,
    pub allowed_hosts: Vec<String>,
    pub allowed_origins: Vec<String>,
    pub max_request_body_bytes: usize,
    pub requests_per_minute: u64,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            resource_url: String::new(),
            issuer: String::new(),
            allowed_hosts: Vec::new(),
            allowed_origins: Vec::new(),
            max_request_body_bytes: DEFAULT_BODY_LIMIT,
            requests_per_minute: 60,
        }
    }
}

impl McpConfig {
    /// Rejects ambiguous URLs, empty host lists, and invalid limits before startup.
    pub fn validate(&self) -> Result<(), McpConfigError> {
        let resource = endpoint(&self.resource_url)?;
        if resource.path() != "/mcp" {
            return Err(McpConfigError::Invalid("resource_url must have path /mcp"));
        }
        endpoint(&self.issuer)?;
        if self.allowed_hosts.is_empty() {
            return Err(McpConfigError::Invalid("allowed_hosts must not be empty"));
        }
        for host in &self.allowed_hosts {
            let authority: Authority = host
                .parse()
                .map_err(|_| McpConfigError::Invalid("invalid allowed host"))?;
            if authority.host().contains('*')
                || host.contains('@')
                || host.to_ascii_lowercase() != *host
            {
                return Err(McpConfigError::Invalid(
                    "allowed hosts must be exact lowercase authorities",
                ));
            }
        }
        for origin in &self.allowed_origins {
            let url = endpoint(origin)?;
            if url.origin().ascii_serialization() != *origin {
                return Err(McpConfigError::Invalid(
                    "allowed origins must be exact origins without a path",
                ));
            }
        }
        if self.max_request_body_bytes == 0 {
            return Err(McpConfigError::Invalid(
                "max_request_body_bytes must be positive",
            ));
        }
        self.quota()?;
        Ok(())
    }

    pub(crate) fn oidc(&self) -> Result<OidcConfig, OidcConfigError> {
        Ok(OidcConfig::new(&self.issuer, &self.resource_url)?
            .with_clock_skew(Duration::ZERO)
            .with_principal_claims(PrincipalClaimMapping::new().client_id_claim("azp")))
    }

    pub(crate) fn quota(&self) -> Result<Quota, QuotaError> {
        Quota::new(self.requests_per_minute, Duration::from_secs(60), 0)
    }

    pub(crate) fn metadata_url(&self) -> Result<String, McpConfigError> {
        let mut url = endpoint(&self.resource_url)?;
        url.set_path("/.well-known/oauth-protected-resource");
        Ok(url.to_string())
    }

    /// Builds RFC 9728 metadata without deriving public URLs from request headers.
    pub fn metadata(&self, scopes: Vec<String>) -> ProtectedResourceMetadata {
        ProtectedResourceMetadata {
            resource: self.resource_url.clone(),
            authorization_servers: vec![self.issuer.clone()],
            bearer_methods_supported: vec!["header".to_owned()],
            scopes_supported: scopes,
        }
    }
}

pub(crate) fn endpoint(value: &str) -> Result<Url, McpConfigError> {
    let url = Url::parse(value).map_err(|_| McpConfigError::Invalid("invalid absolute URL"))?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || url.scheme() == "http" && local)
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(McpConfigError::Invalid(
            "URLs require HTTPS, or loopback HTTP, without credentials, query, or fragment",
        ));
    }
    Ok(url)
}

/// OAuth protected resource discovery document.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProtectedResourceMetadata {
    pub resource: String,
    pub authorization_servers: Vec<String>,
    pub bearer_methods_supported: Vec<String>,
    pub scopes_supported: Vec<String>,
}

/// Startup configuration failure.
#[derive(Debug, thiserror::Error)]
pub enum McpConfigError {
    #[error("invalid MCP configuration: {0}")]
    Invalid(&'static str),
    #[error(transparent)]
    Oidc(#[from] OidcConfigError),
    #[error(transparent)]
    Verification(#[from] baukit_auth::VerificationError),
    #[error(transparent)]
    Quota(#[from] QuotaError),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid() -> McpConfig {
        McpConfig {
            resource_url: "https://mcp.example/mcp".into(),
            issuer: "https://identity.example/realms/product".into(),
            allowed_hosts: vec!["mcp.example".into()],
            ..Default::default()
        }
    }

    #[test]
    fn invalid_urls_allowlists_and_limits_fail_at_startup() {
        let config = valid();
        assert!(config.validate().is_ok());
        for resource in [
            "http://mcp.example/mcp",
            "https://mcp.example/other",
            "https://user:secret@mcp.example/mcp",
            "https://mcp.example/mcp?token=x",
            "https://mcp.example/mcp#fragment",
        ] {
            assert!(
                McpConfig {
                    resource_url: resource.into(),
                    ..config.clone()
                }
                .validate()
                .is_err(),
                "{resource}"
            );
        }
        for hosts in [
            vec![],
            vec!["*.example".into()],
            vec!["user@mcp.example".into()],
            vec!["MCP.EXAMPLE".into()],
        ] {
            assert!(
                McpConfig {
                    allowed_hosts: hosts,
                    ..config.clone()
                }
                .validate()
                .is_err()
            );
        }
        for origin in [
            "null",
            "https://client.example/path",
            "https://client.example/",
            "http://remote.example",
        ] {
            assert!(
                McpConfig {
                    allowed_origins: vec![origin.into()],
                    ..config.clone()
                }
                .validate()
                .is_err(),
                "{origin}"
            );
        }
        assert!(
            McpConfig {
                max_request_body_bytes: 0,
                ..config.clone()
            }
            .validate()
            .is_err()
        );
        assert!(
            McpConfig {
                requests_per_minute: 0,
                ..config
            }
            .validate()
            .is_err()
        );
        for host in ["localhost", "127.0.0.1", "[::1]"] {
            assert!(
                McpConfig {
                    resource_url: format!("http://{host}:8080/mcp"),
                    issuer: format!("http://{host}:8081/realms/product"),
                    allowed_hosts: vec![format!("{host}:8080")],
                    ..Default::default()
                }
                .validate()
                .is_ok()
            );
        }
    }
}
