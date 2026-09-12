# baukit-auth

`baukit-auth` verifies OIDC access tokens against any standards-compliant issuer, issues and checks
personal access tokens, and extracts a `Principal` in Axum handlers. Provider-specific claims never
escape the verifier.

```rust,no_run
use axum::{Router, routing::get};
use baukit_auth::{AuthState, OidcConfig, OidcVerifier, Principal};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let config = OidcConfig::keycloak("https://identity.example.com", "products", "orders-api")?;
let auth = AuthState::new(OidcVerifier::discover(config).await?);

async fn me(principal: Principal) -> String {
    principal.subject().to_owned()
}

let _app: Router = Router::new().route("/me", get(me)).with_state(auth);
# Ok(())
# }
```

## Clerk and WorkOS

The provider adapters apply the token rules that a generic OIDC verifier cannot
infer. They still use the same local JWKS cache and return the same `Principal`.
No provider secret or vendor SDK is needed for request verification.

Clerk session tokens use the instance Frontend API URL as their issuer and may
omit `aud`. Pass every web origin that can obtain a session token. If the token
contains `azp`, `ClerkVerifier` requires an exact allowlist match. It also maps
the Clerk v2 active organization from `o.id`.

```rust,no_run
use baukit_auth::{AuthState, ClerkVerifier};

# fn example() -> Result<(), Box<dyn std::error::Error>> {
let verifier = ClerkVerifier::new(
    "https://example.clerk.accounts.dev",
    ["https://app.example.com", "http://localhost:5173"],
)?;
let _auth = AuthState::new(verifier);
# Ok(())
# }
```

WorkOS AuthKit session tokens also omit `aud`. `WorkOsVerifier` binds them to
one Application through the signed `client_id` claim and maps `org_id`.

```rust,no_run
use baukit_auth::{AuthState, WorkOsVerifier};

# fn example() -> Result<(), Box<dyn std::error::Error>> {
let verifier = WorkOsVerifier::new("client_01ABCDEF")?;
let _auth = AuthState::new(verifier);
# Ok(())
# }
```

Use `WorkOsVerifier::with_issuer` with a custom AuthKit domain. Both adapters
also have `from_jwks_uri` constructors for WorkOS Emulate, private JWKS
proxies, and deterministic tests. Keycloak and other standard OIDC issuers
continue to use `OidcVerifier::discover`.

Any state implementing `FromRef<AuthState>` lets handlers take `Principal` as an extractor. An
unauthenticated request never reaches the handler body.

## Authentication before request middleware

Use `establish_principal` when middleware needs the identity before route extractors run. It verifies
a presented bearer credential and stores `Principal` in request extensions. The route extractor then
reuses that value without a second verification.

```rust
use axum::{Router, middleware, routing::get};
use baukit_auth::{AuthState, IdentityVerifier, Principal, establish_principal};

# fn example(verifier: impl IdentityVerifier + 'static) {
let auth = AuthState::new(verifier);
let app: Router = Router::new()
    .route("/me", get(|principal: Principal| async move {
        principal.subject().to_owned()
    }))
    .with_state(auth.clone())
    .layer(middleware::from_fn_with_state(auth, establish_principal));
# let _ = app;
# }
```

The middleware lets a request with no `Authorization` header continue. This supports anonymous
routes and an inner client-IP safety limit. If the header is present but malformed, invalid, or
expired, the middleware returns the existing `unauthenticated` envelope and does not call inner
middleware. The verifier can be an `OidcVerifier`, `MultiIssuerVerifier`, `ApiTokenVerifier`, or any
product adapter that implements `IdentityVerifier`.

## Only the claims you configured

`OidcVerifier::discover` finds the issuer's JWKS endpoint through standard discovery and validates
signatures, issuer, audience, and expiry. It then maps only the fields named in
`PrincipalClaimMapping` into `Principal`.

Map the provider's OAuth client claim when a route needs a client allowlist:

```rust
use baukit_auth::{OidcConfig, Principal, PrincipalClaimMapping};

# fn example() -> Result<(), Box<dyn std::error::Error>> {
let config = OidcConfig::new("https://identity.example.com", "orders-api")?
    .with_principal_claims(PrincipalClaimMapping::new().client_id_claim("azp"));
# let _ = config;
# Ok(())
# }
fn permits_app_client(principal: &Principal) -> bool {
    principal.issuer().is_some()
        && principal.api_token().is_none()
        && principal.client_id() == Some("orders-mobile")
}
```

The verifier reads the configured claim only after signature, issuer, audience,
expiry and not-before checks pass. An absent or null claim yields `None`; an
empty string or another JSON type fails with `InvalidPrincipalContext`. Values
are preserved exactly, without trimming or case conversion. Unconfigured
claims remain private. API-token and internal principals have no client ID.
Client IDs identify OAuth clients, not users or organizations. A client
allowlist does not prove that a public client is running an unmodified app.

Handing the raw claim set to product code is how a service quietly becomes Keycloak-only. Someone
reads `realm_access.roles` in a handler because it is right there, and swapping the identity provider
becomes a migration instead of a config change. Narrowing at the boundary keeps that decision explicit
and reviewable.

`MultiIssuerVerifier` accepts tokens from several issuers at once, which is what a migration between
providers actually needs.

## Personal access tokens

Interactive OIDC login does not work for a CLI, a cron job, or an MCP server calling the same API.
Those callers need a credential the user creates once and can revoke later.

`ApiTokenService` issues one as a marker plus 32 base62 characters, stores only its SHA-256 digest,
and verifies presented tokens in constant time against that digest.

```rust
use std::sync::Arc;

use baukit_auth::{
    ApiTokenFormat, ApiTokenService, ApiTokenStore, ApiTokenVerifier, AuthState,
    IdentityVerifier, NewApiToken,
};
use uuid::Uuid;

# async fn example(
#     store: Arc<dyn ApiTokenStore>,
#     oidc: Arc<dyn IdentityVerifier>,
#     owner_id: Uuid,
# ) -> Result<(), Box<dyn std::error::Error>> {
let tokens = ApiTokenService::with_format(store, ApiTokenFormat::new("acme_")?);

// Return the secret in the creation response; it cannot be recovered later.
let issued = tokens.issue(owner_id, NewApiToken::new("CI deploy")).await?;
assert!(issued.secret.starts_with("acme_"));

let _auth = AuthState::new(ApiTokenVerifier::new(tokens, oidc));
# Ok(())
# }
```

Three decisions worth naming. Only the digest is stored, so a database dump does not hand over working
credentials and the secret genuinely cannot be shown again. Comparison is constant-time, because a
byte-by-byte compare that returns early leaks the prefix to anyone willing to measure. And the marker
prefix makes a leaked token greppable in logs and scannable in a repository, which is why GitHub's
secret scanning works at all.

`ApiTokenVerifier` wraps the OIDC verifier so one bearer header serves both credential kinds and
handlers stay unaware of which one arrived. Verified token metadata is available through
`Principal::api_token`.

Storage sits behind the `ApiTokenStore` port. The row shape and the ownership join belong to the
product's schema, and a crate that invented its own table would force a second migration path on every
consumer.

Every store operation returns `ApiTokenStoreError`. Use `ApiTokenStoreError::internal(error)` for SQL,
network, and provider failures. The typed error retains its diagnostic string for internal handling,
but `ApiTokenError::Storage` displays only `"API token storage failed"`.

A product policy can return structured public data without putting arbitrary text in an API response:

```rust
use baukit_auth::{ApiTokenPolicyRejection, ApiTokenStoreError};

let rejection = ApiTokenPolicyRejection::new("api_tokens_active_limit_exceeded")?
    .with_detail("maximum", 10)?;
let store_error = ApiTokenStoreError::PolicyRejected(rejection);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Policy codes and detail names are snake_case identifiers of at most 64 ASCII characters. Each
rejection contains at most eight `u32` details. `ApiTokenService` returns these as
`ApiTokenError::PolicyRejected`; all other adapter failures become `ApiTokenError::Storage`.

## Migrating store adapters

This release changes every `ApiTokenStore` result error from `String` to `ApiTokenStoreError`.
Replace `map_err(|error| error.to_string())` with `map_err(ApiTokenStoreError::internal)`. Replace
encoded policy strings such as `limit_exceeded:api_tokens_active:10` with an
`ApiTokenPolicyRejection` code and numeric details. API code should match
`ApiTokenError::PolicyRejected`, then read `code()` and `detail()` instead of parsing a storage error
string. Existing malformed, unknown, hash-mismatched, revoked, and expired credential results do not
change.

## Migrating principal-caching middleware

Replace product middleware that calls `Principal::from_request_parts` with
`middleware::from_fn_with_state(auth, establish_principal)`. Keep the `Principal` extractor on
protected handlers. It now reads the principal established by the middleware and does not verify the
credential again. Requests without a credential still reach anonymous routes. Requests with a bad
credential now stop at the authentication middleware, before an inner anonymous rate-limit bucket.

## Scope

The crate verifies credentials. It does not authorize: roles, permissions, and ownership checks belong
to the product, which is the only place that knows what its resources are. It runs no migrations and
stores nothing. `baukit-test` ships an `InMemoryApiTokenStore` and a `MockOidcServer` so services can
test the whole path without a live provider.
