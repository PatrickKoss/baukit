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

#![deny(missing_docs)]

mod axum_integration;
mod config;
mod verifier;

pub use axum_integration::{AuthRejection, AuthState};
pub use baukit_openapi::{BEARER_AUTH_SCHEME, OpenApiMetadata};
pub use config::{OidcConfig, OidcConfigError, PrincipalClaimMapping, SigningAlgorithm};
pub use verifier::{IdentityVerifier, OidcVerifier, Principal, VerificationError};
