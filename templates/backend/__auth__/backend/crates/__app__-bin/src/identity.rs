use crate::{AuthConfig, AuthProvider};
use baukit_auth::{ClerkVerifier, IdentityVerifier, OidcConfig, OidcVerifier, WorkOsVerifier};
use std::{error::Error, sync::Arc, time::Duration};

pub async fn auth_verifier(
    config: &AuthConfig,
) -> Result<Arc<dyn IdentityVerifier>, Box<dyn Error>> {
    match config.provider {
        AuthProvider::Oidc => {
            let mut oidc =
                OidcConfig::new(&config.issuer, &config.audience)?.with_clock_skew(Duration::ZERO);
            if !config.authorized_parties.is_empty() {
                oidc = oidc.with_allowed_clients(config.authorized_parties.clone())?;
            }
            let verifier = match &config.jwks_uri {
                Some(uri) => OidcVerifier::from_jwks_uri(oidc, uri)?,
                None => OidcVerifier::discover(oidc).await?,
            };
            Ok(Arc::new(verifier))
        }
        AuthProvider::Clerk => {
            let verifier = match &config.jwks_uri {
                Some(uri) => ClerkVerifier::from_jwks_uri(
                    &config.issuer,
                    config.authorized_parties.clone(),
                    uri,
                )?,
                None => ClerkVerifier::new(&config.issuer, config.authorized_parties.clone())?,
            };
            if config.allow_missing_azp {
                return Ok(Arc::new(verifier.allow_missing_authorized_party()));
            }
            Ok(Arc::new(verifier))
        }
        AuthProvider::Workos => {
            let verifier = match &config.jwks_uri {
                Some(uri) => WorkOsVerifier::from_jwks_uri(&config.issuer, &config.client_id, uri)?,
                None => WorkOsVerifier::with_issuer(&config.issuer, &config.client_id)?,
            };
            Ok(Arc::new(verifier))
        }
    }
}
{% if context.mcp %}
pub async fn mcp_verifier(
    auth: &AuthConfig,
    config: &baukit_mcp::McpConfig,
) -> Result<Arc<dyn IdentityVerifier>, Box<dyn Error>> {
    if auth.provider == AuthProvider::Clerk {
        let client_id = config
            .oauth_client_id
            .as_deref()
            .filter(|id| !id.is_empty());
        let jwks = config.jwks_uri.as_deref();
        let verifier: Arc<dyn IdentityVerifier> = match (client_id, jwks) {
            (Some(id), Some(uri)) => Arc::new(baukit_auth::ClerkOAuthVerifier::from_jwks_uri(
                &config.issuer,
                id,
                uri,
            )?),
            (Some(id), None) => Arc::new(baukit_auth::ClerkOAuthVerifier::new(&config.issuer, id)?),
            (None, Some(uri)) => Arc::new(baukit_auth::ClerkMcpVerifier::from_jwks_uri(
                &config.issuer,
                config.required_scopes.clone(),
                uri,
            )?),
            (None, None) => Arc::new(baukit_auth::ClerkMcpVerifier::new(
                &config.issuer,
                config.required_scopes.clone(),
            )?),
        };
        return Ok(verifier);
    }
    let mapping = baukit_auth::PrincipalClaimMapping::new()
        .client_id_claim(if auth.provider == AuthProvider::Oidc {
            "azp"
        } else {
            "client_id"
        })
        .organization_claim("org_id");
    let oidc = OidcConfig::new(&config.issuer, &config.resource_url)?
        .with_clock_skew(Duration::ZERO)
        .with_principal_claims(mapping);
    let jwks = config.jwks_uri.clone().or_else(|| match auth.provider {
        AuthProvider::Oidc => None,
        AuthProvider::Clerk => Some(format!(
            "{}/.well-known/jwks.json",
            config.issuer.trim_end_matches('/')
        )),
        AuthProvider::Workos => Some(format!(
            "{}/oauth2/jwks",
            config.issuer.trim_end_matches('/')
        )),
    });
    let verifier = match jwks {
        Some(uri) => OidcVerifier::from_jwks_uri(oidc, uri)?,
        None => OidcVerifier::discover(oidc).await?,
    };
    Ok(Arc::new(verifier))
}

pub fn mcp_policy(
    auth: &AuthConfig,
    config: &baukit_mcp::McpConfig,
) -> Result<Arc<dyn baukit_mcp::AuthenticationPolicy>, Box<dyn Error>> {
    match (
        &config.introspection_client_id,
        &config.introspection_client_secret,
    ) {
        (None, None) => Ok({{ context.app_crate }}_mcp::authentication_policy()),
        (Some(id), Some(secret)) if auth.provider == AuthProvider::Oidc => {
            Ok(Arc::new(baukit_mcp::KeycloakIntrospectionPolicy::new(
                baukit_mcp::KeycloakIntrospectionConfig::new(&config.issuer, id, secret.clone()),
            )?))
        }
        _ => Err(
            "MCP introspection requires an OIDC Keycloak issuer and both client credentials".into(),
        ),
    }
}
{% endif %}