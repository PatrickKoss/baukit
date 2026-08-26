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

Any state implementing `FromRef<AuthState>` lets handlers take `Principal` as an extractor. An
unauthenticated request never reaches the handler body.

## Only the claims you configured

`OidcVerifier::discover` finds the issuer's JWKS endpoint through standard discovery and validates
signatures, issuer, audience, and expiry. It then maps only the fields named in
`PrincipalClaimMapping` into `Principal`.

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

## Scope

The crate verifies credentials. It does not authorize: roles, permissions, and ownership checks belong
to the product, which is the only place that knows what its resources are. It runs no migrations and
stores nothing. `baukit-test` ships an `InMemoryApiTokenStore` and a `MockOidcServer` so services can
test the whole path without a live provider.
