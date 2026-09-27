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

Neither adapter checks `aud` by default, because neither provider puts one in
its session tokens. When a Clerk JWT template or a WorkOS configuration adds an
audience the product relies on, say so with `with_audiences`. The adapter then
rejects a token whose `aud` is missing or names none of them with
`WrongAudience`. `with_profile_claims` copies named profile claims, described
below.

```rust,no_run
use baukit_auth::{
    ClerkVerifier, IssuerVerifier, MultiIssuerVerifier, OidcConfig, OidcVerifier, WorkOsVerifier,
};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let clerk = ClerkVerifier::new("https://example.clerk.accounts.dev", ["https://app.example.com"])?
    .with_audiences(["orders-api"])?
    .with_profile_claims(["email", "email_verified"]);
let workos = WorkOsVerifier::new("client_01ABCDEF")?;
let keycloak = OidcVerifier::discover(OidcConfig::keycloak(
    "https://identity.example.com",
    "products",
    "orders-api",
)?)
.await?;

let _verifier = MultiIssuerVerifier::from_verifiers([
    IssuerVerifier::from(keycloak),
    clerk.into(),
    workos.into(),
])?;
# Ok(())
# }
```

`MultiIssuerVerifier::from_verifiers` routes on the token's unverified `iss` to
exactly one configured verifier, which then runs its own provider checks. Two
verifiers with the same issuer fail with `MultiIssuerError::DuplicateIssuer`.

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

A rejected bearer token gets an RFC 6750 challenge with one of two fixed descriptions, so a client
can refresh on expiry without reading the body:

```text
WWW-Authenticate: Bearer error="invalid_token", error_description="expired"
WWW-Authenticate: Bearer error="invalid_token", error_description="invalid"
```

Every other verification failure maps to `invalid`. The header never carries claim values, the
issuer, or the verifier's error text. A request without credentials gets a bare `Bearer` challenge.

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

### Scopes and profile claims

`Principal::scopes` holds the verified OAuth scopes. The verifier reads the RFC 9068 `scope` claim by
default and splits it on whitespace. `PrincipalClaimMapping::scope_claim("scp")` reads another
top-level claim instead, and an array of strings works as well as a space-delimited string. A missing
or null claim gives an empty set. Any other shape fails with `InvalidPrincipalContext`, so a malformed
scope claim cannot quietly become "no scopes".

`PrincipalClaimMapping::profile_claims` names the profile claims a product wants, such as `email`,
`email_verified`, `name`, or `picture`. `Principal::profile_claim` returns each one as
`ProfileClaim::String` or `ProfileClaim::Bool`. A selected claim with another JSON type is left out
instead of failing the login, which also means a string `"true"` in `email_verified` never reads as
verified.

```rust
use baukit_auth::{OidcConfig, Principal, PrincipalClaimMapping, ProfileClaim};

# fn example() -> Result<(), Box<dyn std::error::Error>> {
let config = OidcConfig::new("https://identity.example.com", "orders-api")?.with_principal_claims(
    PrincipalClaimMapping::new().profile_claims(["email", "email_verified"]),
);
# let _ = config;
# Ok(())
# }
fn verified_email(principal: &Principal) -> Option<&str> {
    let verified = principal.profile_claim("email_verified").and_then(ProfileClaim::as_bool);
    if verified != Some(true) {
        return None;
    }
    principal.profile_claim("email").and_then(ProfileClaim::as_str)
}
```

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

A token can carry grants: opaque permission strings the product defines and checks.
`NewApiToken::with_grants` sets them at issue time, and `Principal::grants` returns them after
verification. It returns `None` for OIDC and internal principals, so "no grants" and "not an API
token" stay distinct. Each grant is an RFC 6749 scope token of at most 128 bytes, and a token carries
at most 64. Anything else fails with `ApiTokenError::InvalidGrants` before the store is called. Baukit
never interprets a grant, so the names, and any mapping from OIDC scopes to grants, stay in the
product.

```rust
use baukit_auth::{ApiTokenService, NewApiToken, Principal};
use uuid::Uuid;

# async fn example(tokens: ApiTokenService, owner_id: Uuid) -> Result<(), Box<dyn std::error::Error>> {
let issued = tokens
    .issue(owner_id, NewApiToken::new("Nightly export").with_grants(["records:read"]))
    .await?;
# let _ = issued;
# Ok(())
# }
fn can_read(principal: &Principal) -> bool {
    principal.grants().is_some_and(|grants| grants.contains("records:read"))
}
```

Storage sits behind the `ApiTokenStore` port. An adapter must write a record's grants in the same
write as its digest, so a token never exists without them.

## PostgreSQL token store

The `sqlx-postgres` feature adds `PostgresApiTokenStore`. Copy `POSTGRES_API_TOKENS_MIGRATION_SQL`
(`migrations/0001_baukit_auth_api_tokens.sql`) into the product's own migrations; the crate never
migrates on startup. The migration creates `api_tokens` with a SHA-256 `token_hash`, a display
`token_prefix`, `grants TEXT[]`, and the expiry, last-use, and revocation timestamps. Baukit cannot
name the product's owner table, so the product adds the foreign key itself:

```sql
ALTER TABLE api_tokens
    ADD CONSTRAINT api_tokens_owner_fk
    FOREIGN KEY (owner_id) REFERENCES users (id) ON DELETE CASCADE;
```

A product that wants the database to reject unknown grants adds its own `CHECK` on `grants`.

```rust,no_run
# #[cfg(feature = "sqlx-postgres")]
# async fn example(pool: sqlx::PgPool) -> Result<(), Box<dyn std::error::Error>> {
use std::{num::NonZeroU32, sync::Arc};

use baukit_auth::{ApiTokenPolicyRejection, ApiTokenService, PostgresApiTokenStore};

let limit = ApiTokenPolicyRejection::new("api_tokens_active_limit")?.with_detail("maximum", 10)?;
let store = PostgresApiTokenStore::new(pool)
    .with_active_token_limit(NonZeroU32::new(10).ok_or("zero limit")?, limit);
let _tokens = ApiTokenService::new(Arc::new(store));
# Ok(())
# }
```

The store keeps four promises:

- Grants go into the same `INSERT` as the digest. A grant that fails a product `CHECK` leaves no token
  behind.
- With `with_active_token_limit`, the count and the insert run in one transaction under a per-owner
  advisory lock. Concurrent issues cannot overshoot, and tokens that are revoked or expired at the new
  token's `created_at` do not count. Over the limit, `create` returns the configured rejection and
  writes nothing.
- `touch_last_used` only moves `last_used_at` forward, so concurrent requests keep the latest instant.
- `revoke` matches the owner and an unrevoked row, so revoking someone else's token looks like a
  missing one.

`erase_owner_api_tokens` deletes one owner's tokens inside the product's erasure transaction when no
cascading foreign key does it. `purge_inactive_api_tokens` deletes one batch of tokens revoked or
expired before a cutoff and skips rows another writer holds; call it until it returns less than the
batch size.

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

Policy codes are snake_case identifiers and detail names are camelCase identifiers, such as
`activeCount`, each of at most 64 ASCII characters. Detail names become error `details` keys, so
they follow the wire naming rule. Each rejection contains at most eight `u32` details. `ApiTokenService` returns these as
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
to the product, which is the only place that knows what its resources are. Scopes and grants arrive
verified, but deciding what they allow is still the product's job. The only storage it ships is the
optional PostgreSQL token store, and it runs no migrations. `baukit-test` ships an
`InMemoryApiTokenStore` and a `MockOidcServer` so services can test the whole path without a live
provider.
