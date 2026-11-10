use std::{fmt, future::Future, pin::Pin, sync::Arc, time::Duration};

use axum::http::{
    HeaderMap, HeaderValue, StatusCode,
    header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use baukit_egress::{EgressError, EgressRequest, GuardedClient};
use serde::Deserialize;
use url::{Url, form_urlencoded};
use zeroize::Zeroizing;

use crate::{OidcConfig, OidcVerifier, VerificationError};

use super::config::{BrowserSessionConfig, BrowserSessionConfigError};

pub(crate) type IdpFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, IdpError>> + Send + 'a>>;

/// Tokens from one token-endpoint response.
pub(crate) struct TokenSet {
    pub(crate) access_token: Zeroizing<String>,
    pub(crate) expires_in: Duration,
    pub(crate) refresh_token: Option<Zeroizing<String>>,
    pub(crate) refresh_expires_in: Option<Duration>,
    pub(crate) id_token: Option<Zeroizing<String>>,
}

/// IdP token client port.
pub(crate) trait IdentityProvider: Send + Sync {
    fn exchange<'a>(
        &'a self,
        code: &'a str,
        code_verifier: &'a str,
        redirect_uri: &'a str,
    ) -> IdpFuture<'a, TokenSet>;
    fn refresh<'a>(&'a self, refresh_token: &'a str) -> IdpFuture<'a, TokenSet>;
    /// Revokes a refresh token per RFC 7009. A no-op without a revocation endpoint.
    fn revoke<'a>(&'a self, refresh_token: &'a str) -> IdpFuture<'a, ()>;
    /// Verifies an ID token from a direct token response and returns its subject.
    fn subject<'a>(&'a self, id_token: &'a str) -> IdpFuture<'a, String>;
    fn end_session_url(&self, id_token: Option<&str>) -> Option<String>;
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum IdpError {
    #[error("the IdP rejected the grant")]
    InvalidGrant,
    #[error("the IdP is unavailable")]
    Unavailable(#[source] EgressError),
    #[error("the IdP signing keys are unavailable")]
    KeysUnavailable(#[source] VerificationError),
    #[error("the IdP response is invalid: {0}")]
    InvalidResponse(&'static str),
    #[error("the ID token failed verification")]
    InvalidIdToken(#[source] VerificationError),
}

/// Checks ID tokens against the issuer JWKS with the shared [`OidcVerifier`].
pub(crate) struct IdTokenVerifier {
    verifier: OidcVerifier,
}

impl IdTokenVerifier {
    pub(crate) fn new(
        issuer: &str,
        client_id: &str,
        jwks_uri: &str,
        request_timeout: Duration,
    ) -> Result<Self, IdpDiscoveryError> {
        let config = OidcConfig::new(issuer, client_id)
            .and_then(|config| config.with_client_id(client_id))
            .and_then(|config| config.with_request_timeout(request_timeout))
            .map_err(|_| IdpDiscoveryError::InvalidDocument)?;
        let verifier = OidcVerifier::from_jwks_uri(config, jwks_uri)
            .map_err(|_| IdpDiscoveryError::InvalidDocument)?;
        Ok(Self { verifier })
    }

    pub(crate) async fn subject(&self, id_token: &str) -> Result<String, IdpError> {
        match self.verifier.verify_exchanged_id_token(id_token).await {
            Ok(principal) => Ok(principal.subject().to_owned()),
            Err(error) if error.is_unavailable() => Err(IdpError::KeysUnavailable(error)),
            Err(error) => Err(IdpError::InvalidIdToken(error)),
        }
    }
}

#[derive(Deserialize)]
struct DiscoveryDocument {
    issuer: String,
    token_endpoint: String,
    jwks_uri: String,
    revocation_endpoint: Option<String>,
    end_session_endpoint: Option<String>,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
    refresh_expires_in: Option<u64>,
    id_token: Option<String>,
}

/// OIDC token client that sends every request through [`GuardedClient`].
pub(crate) struct OidcTokenClient {
    client: GuardedClient,
    id_tokens: IdTokenVerifier,
    issuer: String,
    client_id: String,
    client_secret: Option<Zeroizing<String>>,
    token_endpoint: Url,
    revocation_endpoint: Option<Url>,
    end_session_endpoint: Option<Url>,
    post_logout_redirect_uri: Option<String>,
}

impl fmt::Debug for OidcTokenClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OidcTokenClient")
            .field("issuer", &self.issuer)
            .field("client_id", &self.client_id)
            .finish_non_exhaustive()
    }
}

/// Failure to discover the IdP's endpoints at startup.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IdpDiscoveryError {
    /// The configuration cannot form an egress client.
    #[error(transparent)]
    Config(#[from] BrowserSessionConfigError),
    /// The egress client could not be built.
    #[error("could not build the IdP egress client")]
    Client(#[source] baukit_egress::EgressClientError),
    /// The discovery request failed.
    #[error("OIDC discovery failed")]
    Request(#[source] EgressError),
    /// The discovery document is not valid JSON or lacks a token endpoint.
    #[error("OIDC discovery document is invalid")]
    InvalidDocument,
    /// The discovery document names another issuer.
    #[error("OIDC discovery document names another issuer")]
    IssuerMismatch,
}

impl OidcTokenClient {
    pub(crate) async fn discover(
        config: &BrowserSessionConfig,
    ) -> Result<Arc<Self>, IdpDiscoveryError> {
        let client =
            GuardedClient::new(config.egress_options()?).map_err(IdpDiscoveryError::Client)?;
        let idp_base = config.idp_base();
        let mut discovery_url = idp_base.clone();
        discovery_url.set_path(&format!(
            "{}/.well-known/openid-configuration",
            idp_base.path().trim_end_matches('/')
        ));
        let response = client
            .execute(EgressRequest::get(discovery_url).with_headers(json_accept()))
            .await
            .map_err(IdpDiscoveryError::Request)?;
        let document: DiscoveryDocument = serde_json::from_slice(response.body())
            .map_err(|_| IdpDiscoveryError::InvalidDocument)?;
        if document.issuer != config.issuer_text {
            return Err(IdpDiscoveryError::IssuerMismatch);
        }
        let parse =
            |value: String| Url::parse(&value).map_err(|_| IdpDiscoveryError::InvalidDocument);
        let jwks_uri = parse(document.jwks_uri)?;
        if jwks_uri.origin() != idp_base.origin() {
            return Err(IdpDiscoveryError::InvalidDocument);
        }
        let id_tokens = IdTokenVerifier::new(
            &document.issuer,
            &config.client_id,
            jwks_uri.as_str(),
            config.request_timeout,
        )?;
        Ok(Arc::new(Self {
            client,
            id_tokens,
            issuer: document.issuer,
            client_id: config.client_id.clone(),
            client_secret: config.client_secret.clone(),
            token_endpoint: parse(document.token_endpoint)?,
            revocation_endpoint: document.revocation_endpoint.map(parse).transpose()?,
            end_session_endpoint: document.end_session_endpoint.map(parse).transpose()?,
            post_logout_redirect_uri: config.post_logout_redirect_uri.clone(),
        }))
    }

    fn form_request(&self, url: Url, fields: &[(&str, &str)]) -> EgressRequest {
        let mut body = form_urlencoded::Serializer::new(String::new());
        for (name, value) in fields {
            body.append_pair(name, value);
        }
        body.append_pair("client_id", &self.client_id);
        let mut headers = json_accept();
        headers.insert(
            CONTENT_TYPE,
            HeaderValue::from_static("application/x-www-form-urlencoded"),
        );
        if let Some(authorization) = self.basic_authorization() {
            headers.insert(AUTHORIZATION, authorization);
        }
        EgressRequest::post(url)
            .with_headers(headers)
            .with_body(Zeroizing::new(body.finish()).as_bytes().to_vec())
    }

    fn basic_authorization(&self) -> Option<HeaderValue> {
        let secret = self.client_secret.as_ref()?;
        let encode =
            |value: &str| form_urlencoded::byte_serialize(value.as_bytes()).collect::<String>();
        let credentials = Zeroizing::new(format!("{}:{}", encode(&self.client_id), encode(secret)));
        let mut value = HeaderValue::from_str(&format!(
            "Basic {}",
            STANDARD.encode(credentials.as_bytes())
        ))
        .ok()?;
        value.set_sensitive(true);
        Some(value)
    }

    async fn token_request(&self, fields: &[(&str, &str)]) -> Result<TokenSet, IdpError> {
        let request = self.form_request(self.token_endpoint.clone(), fields);
        let response = self.client.execute(request).await.map_err(token_error)?;
        let tokens: TokenResponse = serde_json::from_slice(response.body())
            .map_err(|_| IdpError::InvalidResponse("token response is not valid JSON"))?;
        if tokens.access_token.is_empty() || tokens.expires_in == 0 {
            return Err(IdpError::InvalidResponse(
                "token response lacks an access token",
            ));
        }
        Ok(TokenSet {
            access_token: Zeroizing::new(tokens.access_token),
            expires_in: Duration::from_secs(tokens.expires_in),
            refresh_token: tokens.refresh_token.map(Zeroizing::new),
            refresh_expires_in: tokens
                .refresh_expires_in
                .filter(|seconds| *seconds > 0)
                .map(Duration::from_secs),
            id_token: tokens.id_token.map(Zeroizing::new),
        })
    }
}

fn json_accept() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers
}

fn token_error(error: EgressError) -> IdpError {
    match &error {
        EgressError::Status { status, .. } if *status == StatusCode::BAD_REQUEST => {
            IdpError::InvalidGrant
        }
        _ => IdpError::Unavailable(error),
    }
}

impl IdentityProvider for OidcTokenClient {
    fn exchange<'a>(
        &'a self,
        code: &'a str,
        code_verifier: &'a str,
        redirect_uri: &'a str,
    ) -> IdpFuture<'a, TokenSet> {
        Box::pin(async move {
            self.token_request(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("code_verifier", code_verifier),
                ("redirect_uri", redirect_uri),
            ])
            .await
        })
    }

    fn refresh<'a>(&'a self, refresh_token: &'a str) -> IdpFuture<'a, TokenSet> {
        Box::pin(async move {
            self.token_request(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ])
            .await
        })
    }

    fn revoke<'a>(&'a self, refresh_token: &'a str) -> IdpFuture<'a, ()> {
        Box::pin(async move {
            let Some(endpoint) = self.revocation_endpoint.clone() else {
                return Ok(());
            };
            let request = self.form_request(
                endpoint,
                &[
                    ("token", refresh_token),
                    ("token_type_hint", "refresh_token"),
                ],
            );
            self.client
                .execute(request)
                .await
                .map_err(IdpError::Unavailable)?;
            Ok(())
        })
    }

    fn subject<'a>(&'a self, id_token: &'a str) -> IdpFuture<'a, String> {
        Box::pin(self.id_tokens.subject(id_token))
    }

    fn end_session_url(&self, id_token: Option<&str>) -> Option<String> {
        let mut url = self.end_session_endpoint.clone()?;
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("client_id", &self.client_id);
            if let Some(id_token) = id_token {
                query.append_pair("id_token_hint", id_token);
            }
            if let Some(redirect) = &self.post_logout_redirect_uri {
                query.append_pair("post_logout_redirect_uri", redirect);
            }
        }
        Some(url.into())
    }
}
