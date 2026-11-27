# ADR 0017: Bearer tokens are verified by baukit-auth per provider profile, and Keycloak is for development only

## Status
Accepted, 2026-11-27.

## Context
Products sign users in with Clerk, WorkOS or a standard OIDC issuer. Each issues tokens with different claims. Clerk session tokens carry no `aud` and set `azp` from the request origin. Keycloak access tokens list several audiences. One verifier with one rule set either rejects valid tokens or accepts tokens meant for another client.

Running Keycloak in production also means operating it: upgrades, realm migrations and an operator with large CRDs. No product needs that once Clerk and WorkOS cover production sign-in.

## Decision
`baukit-auth` verifies bearer tokens behind the `IdentityVerifier` port. `OidcVerifier` takes an `OidcConfig`. `ClerkVerifier`, `ClerkOAuthVerifier`, `ClerkMcpVerifier` and `WorkOsVerifier` in `providers.rs` wrap it with provider defaults. The token profile picks the client check in `verifier/claims.rs`:

- OIDC access tokens are checked against the audience. With `with_allowed_clients`, `azp` must be in that list. Without it, a token with several audiences needs a string `azp`, and a single-audience token needs none. Generated products set `auth.authorized_parties` in `config/local.toml`.
- Clerk tokens need an `azp` from the allowlist. `ClerkVerifier::allow_missing_authorized_party` accepts tokens without one, as native apps send.
- ID tokens must name the client in `aud`, and a present `azp` must equal it.

Keycloak runs in the product compose stack, E2E and QA. `deploy/platform/keycloak` is an optional base for shared non-production clusters. Production has no Keycloak.

## Enforced by
- `profile_validation_matches_reference_table` (P-05, proptest) and `access_tokens_with_several_audiences_check_azp_only_with_an_allow_list` in `rust/crates/baukit-auth/src/verifier/claims_tests.rs`. Gate step `rust-tests`.
- `trusted_proxy_hops_are_accepted_outside_local_up_to_the_bound` and `rate_limit_defaults_ignore_forwarded_for_and_bucket_ipv6_by_64` in `rust/crates/baukit-config/src/tests.rs` pin the proxy default.
- Nothing checks that production overlays carry no Keycloak. A test in `platform-validate` that rejects the `keycloak` base under a production overlay would.

## Consequences
Products choose a provider and get its claim rules. A multi-audience OIDC product must list its clients to restrict `azp`. Products that want a self-hosted IdP in production run it themselves.

## History
- 2026-10-03, `c962ca19`: access-token client restrictions made opt-in.
- 2026-11-05, `8c01a4bb`: the production Keycloak base, its CRDs and realm jobs were removed. Production signs in with Clerk or WorkOS.
- 2026-11-07, `273da59d`: Clerk tokens without `azp` rejected unless opted in.
- 2026-11-21, `3ab09ce0`: an optional Keycloak base returned for shared non-production clusters.
- 2026-11-25, `fe8a82c2`: multi-audience access tokens required an `azp` equal to the configured client ID. F-25 reverted this on 2026-11-26 in `bfb8781b`, because Keycloak's default `aud: ["app-backend", "account"]` failed without an allowlist. `e45d5b59` kept the rule that such tokens carry a string `azp`.
- 2026-11-25, `3ee7df1f`: the `trusted_proxy_hops` default in `baukit_config::RateLimitConfig` went from 1 to 0, so `X-Forwarded-For` is ignored until a deployment states its proxy count.
