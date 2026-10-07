# Changelog

All notable changes to `baukit-auth` are documented here.

## [Unreleased]

## [0.8.0] - 2026-10-07

## [0.7.4] - 2026-10-06

## [0.7.3] - 2026-10-05

- Use aws-lc-rs for cryptographic operations instead of a second ring dependency. Keep the supported algorithms and wire formats.

## [0.7.2] - 2026-10-04

## [0.7.1] - 2026-10-04

## [0.7.0] - 2026-10-04

### Added

- Add `OidcVerifier::verify_id_token(token, expected_nonce)` to require the
  login request's nonce on an ID token. Missing, malformed, and mismatched
  nonces return `VerificationError::WrongNonce`.
- Add `constant_time_eq` for secret bytes, using `subtle` 2.6.1. Unequal lengths
  return false; equal-length inputs do not stop at a differing byte. API token
  digest verification now uses this helper instead of a handwritten loop.

### Changed

- Use caret requirements for third-party Rust dependencies so products can take compatible
  updates. Keep Baukit crate versions exact.

- Generic OIDC access tokens with several audiences must carry a string `azp`.
  Client restrictions remain opt-in through `with_allowed_clients`, which
  requires `azp` from that list on every access token. Single-audience tokens
  keep their existing claim-mapping behavior without an allowlist.
- ID-token verification requires an explicit `with_client_id` and that client
  in `aud`. Several audiences require `azp` equal to the client, and any present
  `azp` must equal it. The API audience does not set the ID-token client. Clerk
  and WorkOS keep their access-token provider checks.

### Fixed

- Select aws-lc-rs for sqlx TLS so rustls has one crypto provider. Products
  must use sqlx `tls-rustls-aws-lc-rs` instead of `tls-rustls`, including dev
  dependencies, and disable the `ring` default in testcontainers and
  testcontainers-modules.

## [0.6.0] - 2026-10-02

### Changed

- Moved to reqwest 0.13 with its `rustls` backend. HTTPS clients now verify servers against the
  operating system's trust store through rustls-platform-verifier instead of the bundled
  webpki roots, so a container image needs CA certificates (distroless `cc` ships them). The
  TLS crypto provider is aws-lc-rs instead of ring.

## [0.5.2] - 2026-09-30

## [0.5.1] - 2026-09-29

## [0.5.0] - 2026-09-28

### Added

- Add the `sqlx-postgres` feature with `PostgresApiTokenStore`, an
  `ApiTokenStore` over the `api_tokens` table in
  `POSTGRES_API_TOKENS_MIGRATION_SQL`. The constant is available without the
  feature. Grants go into the same `INSERT` as the digest.
  `with_active_token_limit` counts and inserts under a per-owner advisory lock.
  `touch_last_used` never moves `last_used_at` backwards, and `revoke` matches
  the owner. Products copy the migration and add their own owner foreign key.
- Add `erase_owner_api_tokens` for owner erasure without a cascading foreign
  key, and `purge_inactive_api_tokens` for batched retention of revoked and
  expired tokens. Both need the `sqlx-postgres` feature.
- Add opaque API token grants. `NewApiToken::with_grants` sets them,
  `ApiTokenRecord::grants` and `ApiToken::grants` carry them, and
  `Principal::grants` returns them for API-token principals. Each grant is an
  RFC 6749 scope token of at most `MAX_API_TOKEN_GRANT_LENGTH` (128) bytes, and
  a token carries at most `MAX_API_TOKEN_GRANTS` (64). Anything else fails with
  the new `ApiTokenError::InvalidGrants`.
- Add `Principal::scopes`, read from the verified `scope` claim by default.
  `PrincipalClaimMapping::scope_claim` reads another claim. A space-delimited
  string and an array of strings both work.
- Add `PrincipalClaimMapping::profile_claims`, `Principal::profile_claims`,
  `Principal::profile_claim`, and `ProfileClaim` for caller-selected verified
  string and boolean claims such as `email` and `email_verified`.
- Add `ClerkVerifier::with_audiences` and `WorkOsVerifier::with_audiences` for
  products that need an `aud` check. Both adapters still skip the check by
  default. Add `with_profile_claims` and `issuer` to both, and
  `OidcVerifier::issuer`.
- Add `IssuerVerifier` and `MultiIssuerVerifier::from_verifiers`, so one
  multi-issuer verifier routes OIDC, Clerk, and WorkOS tokens.

### Changed

- Break: `ApiToken`, `ApiTokenRecord`, and `NewApiToken` gain a public
  `grants: BTreeSet<String>` field. Struct literals must set it. Store adapters
  must persist `ApiTokenRecord::grants` in the same write as the digest and
  return them on `ApiToken`.
- Break: `ApiTokenError` gains `InvalidGrants`. Exhaustive matches must handle
  it.
- Break: `PrincipalClaimMapping::new()` and `default()` now map `scope`, so
  OIDC, Clerk, and WorkOS verification reads it. A token whose `scope` is
  neither a string, an array of non-empty strings, nor null fails with
  `InvalidPrincipalContext`, where it used to verify.
- Break: `AuthRejection` challenges use the RFC 6750 `error_description`
  parameter instead of the nonstandard `hint`. Expired tokens now get
  `Bearer error="invalid_token", error_description="expired"` and every other
  rejected token gets `error_description="invalid"`. Clients that parsed
  `hint=` must read `error_description=`.
- Break: `ApiTokenPolicyRejection::with_detail` accepts camelCase detail names only, such as
  `activeCount`, because they become error `details` keys. A snake_case name such as
  `active_count` now returns `InvalidDetailName`. Codes stay snake_case.

## [0.4.0] - 2026-09-12

### Added

- Add `ClerkVerifier` and `WorkOsVerifier` provider adapters. Clerk verification
  checks an `azp` allowlist and maps the v2 `o.id` organization. WorkOS
  verification requires the configured Application `client_id` and maps
  `org_id`. Both accept provider session tokens without an `aud` claim and
  support explicit JWKS endpoints for tests, proxies, and WorkOS Emulate.
- Add optional `Principal::client_id()` and
  `PrincipalClaimMapping::client_id_claim()` for client-restricted product
  policies. The verifier maps only the configured claim after token checks.
  Missing client identity stays `None`; malformed mapped claims fail verification.

## [0.3.0] - 2026-09-04

### Added

- Add validated `ApiTokenPolicyRejection` codes with at most eight numeric
  details so adapters can return safe policy decisions.
- Add `establish_principal` for Axum compositions that need a verified
  `Principal` before rate limiting or other request middleware. Missing
  credentials continue without a principal; presented invalid credentials use
  the existing authentication envelope.

### Changed

- Change every `ApiTokenStore` operation from `String` failures to
  `ApiTokenStoreError`. Internal adapter diagnostics now map to the generic
  `ApiTokenError::Storage`; policy rejections map to
  `ApiTokenError::PolicyRejected`.

### Migration

- Update each product adapter to return `ApiTokenStoreError`. Wrap SQL and
  provider errors with `ApiTokenStoreError::internal`. Replace encoded policy
  strings with `ApiTokenPolicyRejection` and update API mappings to inspect its
  code and numeric details.
- Replace product-owned principal-caching middleware with
  `middleware::from_fn_with_state(auth, establish_principal)`. Protected route
  extractors remain unchanged and reuse the cached principal.

## [0.2.1] - 2026-09-03

## [0.2.0] - 2026-09-03

### Fixed

- Keep API token validation compatible with current stable Clippy's
  `nonminimal_bool` lint.

## [0.1.2] - 2026-09-01

## [0.1.1] - 2026-09-01

## [0.1.0] - 2026-08-25

### Added

- First public release of `baukit-auth`.
