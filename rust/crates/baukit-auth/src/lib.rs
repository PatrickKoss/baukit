//! Provider-neutral OIDC access-token verification for Baukit services.
//!
//! [`OidcVerifier`] discovers an issuer's JWKS endpoint, validates signed JWTs,
//! and maps only configured identity fields into [`Principal`]. [`ClerkVerifier`]
//! and [`WorkOsVerifier`] add the provider-specific claim checks used by their
//! session tokens without exposing those claims to product handlers.
//! [`establish_principal`] verifies a presented bearer credential before inner
//! middleware runs. Axum handlers can then extract `Principal` without a second
//! verification. Provider-specific claims remain private to the verifier.
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
//! Storage sits behind the [`ApiTokenStore`] port. The optional
//! `sqlx-postgres` feature adds a PostgreSQL adapter and a reference migration,
//! [`POSTGRES_API_TOKENS_MIGRATION_SQL`]; the product still owns the ownership
//! join. Tokens can carry opaque product-defined grants, stored in the same
//! write as the digest. Wrapping the OIDC verifier in [`ApiTokenVerifier`]
//! makes one bearer header serve both credential kinds. The [`Principal`]
//! extractor exposes verified token metadata through [`Principal::api_token`]
//! and the grants through [`Principal::grants`]. Adapters return
//! [`ApiTokenStoreError`], which separates private internal failures from safe
//! structured [`ApiTokenPolicyRejection`] values.
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
#[cfg(feature = "sqlx-postgres")]
mod postgres;
mod providers;
mod verifier;

pub use api_token::{
    ApiToken, ApiTokenError, ApiTokenFormat, ApiTokenFormatError, ApiTokenPolicyRejection,
    ApiTokenPolicyRejectionError, ApiTokenRecord, ApiTokenService, ApiTokenStore,
    ApiTokenStoreError, ApiTokenStoreFuture, ApiTokenVerifier, DEFAULT_API_TOKEN_MARKER,
    IssuedApiToken, MAX_API_TOKEN_GRANT_LENGTH, MAX_API_TOKEN_GRANTS, NewApiToken, StoredApiToken,
    hash_api_token,
};
pub use axum_integration::{AuthRejection, AuthState, establish_principal};
pub use baukit_openapi::{BEARER_AUTH_SCHEME, OpenApiMetadata};
pub use config::{OidcConfig, OidcConfigError, PrincipalClaimMapping, SigningAlgorithm};
#[cfg(feature = "sqlx-postgres")]
pub use postgres::{PostgresApiTokenStore, erase_owner_api_tokens, purge_inactive_api_tokens};
pub use providers::{ClerkVerifier, ProviderVerifierError, WorkOsVerifier};
pub use verifier::{
    IdentityVerifier, IssuerVerifier, MultiIssuerError, MultiIssuerVerifier, OidcVerifier,
    Principal, ProfileClaim, VerificationError,
};

/// Reference PostgreSQL schema for the `sqlx-postgres` feature's `PostgresApiTokenStore`.
///
/// Copy this SQL into a product migration; do not execute it dynamically during
/// application startup. Then add the product's owner foreign key with
/// `ON DELETE CASCADE`, as the file's header describes.
pub const POSTGRES_API_TOKENS_MIGRATION_SQL: &str =
    include_str!("../migrations/0001_baukit_auth_api_tokens.sql");

// Compiles the README's examples so they cannot drift from the API.
#[doc = include_str!("../README.md")]
#[cfg(doctest)]
struct ReadmeDoctests;
