//! Provider-neutral OIDC access-token verification for Baukit services.
//!
//! [`OidcVerifier`] discovers an issuer's JWKS endpoint, validates signed JWTs,
//! and maps only configured identity fields into [`Principal`]. Axum handlers
//! can extract `Principal` when their state implements `FromRef` for
//! [`AuthState`]. Provider-specific claims remain private to the verifier.
//!
//! ```no_run
//! use axum::{Router, routing::get};
//! use baukit_auth::{AuthState, OidcConfig, OidcVerifier, Principal};
//! use baukit_openapi::OpenApiMetadata;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let config = OidcConfig::keycloak(
//!     "https://identity.example.com",
//!     "products",
//!     "orders-api",
//! )?;
//! let auth = AuthState::new(OidcVerifier::discover(config).await?);
//! async fn me(principal: Principal) -> String {
//!     principal.subject().to_owned()
//! }
//! let _app: Router = Router::new().route("/me", get(me)).with_state(auth);
//! let _metadata = OpenApiMetadata::new("Orders", "1.0.0", "Orders API").bearer_auth();
//! # Ok(())
//! # }
//! ```
//!
//! # Personal access tokens
//!
//! Interactive OIDC logins do not work for a CLI, a cron job, or an MCP server
//! talking to the same API. Those callers need a long-lived credential the user
//! creates once and can revoke later. [`ApiTokenService`] issues one as a
//! marker plus 32 base62 characters, stores only its SHA-256 digest, and
//! verifies presented tokens in constant time against that digest.
//!
//! Storage stays product-local behind the [`ApiTokenStore`] port, because the
//! row shape and the ownership join belong to the product's schema. Wrapping
//! the OIDC verifier in [`ApiTokenVerifier`] makes one bearer header serve both
//! credential kinds. The [`Principal`] extractor exposes verified token
//! metadata through [`Principal::api_token`].
//!
//! ```
//! use std::sync::Arc;
//!
//! use baukit_auth::{
//!     ApiTokenFormat, ApiTokenService, ApiTokenStore, ApiTokenVerifier, AuthState,
//!     IdentityVerifier, NewApiToken,
//! };
//! use uuid::Uuid;
//!
//! # async fn example(
//! #     store: Arc<dyn ApiTokenStore>,
//! #     oidc: Arc<dyn IdentityVerifier>,
//! #     owner_id: Uuid,
//! # ) -> Result<(), Box<dyn std::error::Error>> {
//! let tokens = ApiTokenService::with_format(store, ApiTokenFormat::new("acme_")?);
//!
//! // Return the secret in the creation response; it cannot be recovered later.
//! let issued = tokens.issue(owner_id, NewApiToken::new("CI deploy")).await?;
//! assert!(issued.secret.starts_with("acme_"));
//!
//! let _auth = AuthState::new(ApiTokenVerifier::new(tokens, oidc));
//! # Ok(())
//! # }
//! ```

#![deny(missing_docs)]

mod api_token;
mod axum_integration;
mod config;
mod verifier;

pub use api_token::{
    ApiToken, ApiTokenError, ApiTokenFormat, ApiTokenFormatError, ApiTokenRecord, ApiTokenService,
    ApiTokenStore, ApiTokenStoreFuture, ApiTokenVerifier, DEFAULT_API_TOKEN_MARKER, IssuedApiToken,
    NewApiToken, StoredApiToken, hash_api_token,
};
pub use axum_integration::{AuthRejection, AuthState};
pub use baukit_openapi::{BEARER_AUTH_SCHEME, OpenApiMetadata};
pub use config::{OidcConfig, OidcConfigError, PrincipalClaimMapping, SigningAlgorithm};
pub use verifier::{
    IdentityVerifier, MultiIssuerError, MultiIssuerVerifier, OidcVerifier, Principal,
    VerificationError,
};

// Compiles the README's examples so they cannot drift from the API.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
struct ReadmeDoctests;
