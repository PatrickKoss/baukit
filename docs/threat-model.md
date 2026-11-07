# Threat model

Baukit foundation, written with `agent-skills/skills/baukit-threat-model`. Last reviewed: 2026-11-07, on top of commit d8fdd25a.

Baukit is a library, so this model covers the controls the crates, packages, templates and edge config provide, and the threats they cover for a product that uses them as generated. Each product keeps its own `docs/threat-model.md` for routes, data and integrations it adds. Gaps S-01 to S-11 come from the security catalog in `docs/quality-security-plan.md`.

## Data classes

| Class | Kind | Stores | Retention and erasure |
|---|---|---|---|
| Identity claims (subject, email, org) | personal | PostgreSQL via `baukit-auth` principal, logs as hashed or bounded fields | `baukit-erasure`, `baukit-data-subject` |
| Access and refresh tokens | credential | browser storage via `@baukit/auth-web` (S-01), native secure store via `@baukit/auth-native` | token lifetime |
| API tokens | credential | PostgreSQL, hashed (`baukit-auth/src/api_token.rs`) | revocation and expiry |
| Third-party secrets | credential | PostgreSQL, AES-256-GCM (`baukit-credential-vault`) | deleted with the connection |
| Synced user content | product-defined | PostgreSQL, device SQLite and IndexedDB (`@baukit/data-contracts-*`) | `baukit-sync` purge horizons |
| Media | content | object storage behind the njs edge | product-defined |
| Telemetry and analytics | operational | `baukit-telemetry`, `@baukit/analytics-*` | collector retention |

## Trust boundaries

| Boundary | Entry points | Identity |
|---|---|---|
| browser and native app to API | `baukit-http` router, sync push and pull | OIDC, Clerk or WorkOS bearer, API token |
| MCP client to MCP endpoint | `baukit-mcp` HTTP transport | bearer or API token, Origin allowlist |
| client to njs media edge | grant query string | HMAC grant |
| IdP to API | discovery and JWKS fetch | issuer URL |
| API to third parties | `baukit-egress` `GuardedClient` | product credentials |
| inbound webhooks and suite hub | `baukit-core` webhook signatures, `baukit-suite` receiver | shared secret or peer signature |

## Threats

| ID | Boundary | STRIDE | Threat | Control | Test | Status |
|---|---|---|---|---|---|---|
| B-01 | IdP to API | S | Unsigned or `alg: none` token accepted | asymmetric algorithm allowlist, kid required (`rust/crates/baukit-auth/src/config.rs`, `rust/crates/baukit-auth/src/verifier.rs`) | `rust/crates/baukit-auth/src/verifier.rs::unsigned_and_malformed_tokens_are_rejected_before_key_lookup` | covered |
| B-02 | IdP to API | S | Token from another issuer, audience or expired | issuer, audience and expiry checks per provider profile (`rust/crates/baukit-auth/src/verifier.rs`) | `rust/crates/baukit-auth/tests/providers.rs::provider_sessions_check_issuer_audience_expiry_and_identity_context` | covered |
| B-03 | IdP to API | S | Clerk session token used where the OAuth client is required | Clerk OAuth profile (`rust/crates/baukit-auth/src/providers.rs`) | `rust/crates/baukit-auth/tests/providers.rs::clerk_oauth_requires_the_dedicated_client_and_rejects_session_tokens` | covered |
| B-04 | browser to API | S, E | Revoked or expired API token keeps working | per-request revocation and expiry check, hashed constant-time compare (`rust/crates/baukit-auth/src/api_token.rs`, `rust/crates/baukit-auth/src/secret.rs`) | `rust/crates/baukit-mcp/tests/http.rs::api_tokens_are_rechecked_for_revocation_and_expiry_on_every_request` | covered |
| B-05 | MCP client to MCP endpoint | S | Cross-site page drives the MCP endpoint with ambient credentials | exact Host and Origin allowlist (`rust/crates/baukit-mcp/src/security.rs`) | `rust/crates/baukit-mcp/src/security.rs::host_and_origin_are_exact_and_duplicates_fail_closed` | covered |
| B-06 | MCP client to MCP endpoint | E | API token grant reaches tools beyond its scope | grant filtering, policies only narrow (`rust/crates/baukit-mcp/src/policy.rs`) | `rust/crates/baukit-mcp/tests/http.rs::api_token_grants_filter_tools_and_policies_can_only_narrow` | covered |
| B-07 | browser to API | D | Unbounded request bodies | global and per-route body limits (`rust/crates/baukit-http/src/middleware.rs`) | `rust/crates/baukit-http/src/tests.rs::configured_body_limit_has_the_body_too_large_class`, `::route_body_limit_has_the_body_too_large_class` | covered |
| B-08 | browser to API | D | Request floods per identity or address | `baukit-ratelimit` layer with trusted-hop XFF (`rust/crates/baukit-ratelimit/src/axum_layer.rs`) | `rust/crates/baukit-ratelimit/src/axum_layer.rs::xff_resolution_honors_trusted_hop_count`, `::middleware_principals_use_separate_identity_buckets` | covered |
| B-09 | browser to API | I | Error bodies echo input or internal causes | `ApiError` envelope, internal causes logged only (`rust/crates/baukit-http/src/error.rs`) | `rust/crates/baukit-http/src/tests.rs::camel_case_bodies_reject_snake_case_names_without_echoing_them` | covered |
| B-10 | API to third parties | I, E | SSRF to loopback, private ranges or metadata endpoints; redirect to internal host; DNS rebinding | `GuardedClient` address policy, pinned resolution, no redirects (`rust/crates/baukit-egress/src/client.rs`, `rust/crates/baukit-egress/src/policy.rs`) | `rust/crates/baukit-egress/tests/guarded_client.rs::the_default_policy_blocks_a_loopback_answer_before_connecting`, `::a_changed_dns_answer_never_reaches_a_connection`, `::redirects_are_returned_and_never_followed`, `rust/crates/baukit-egress/tests/address_policy_vectors.rs` | covered |
| B-11 | API to third parties | I | Credentials or URLs leak through egress telemetry | redacted spans and errors (`rust/crates/baukit-egress/src/error.rs`) | `rust/crates/baukit-egress/tests/telemetry_redaction.rs` | covered |
| B-12 | API at rest | T, I | Stored third-party secret read or swapped between records | AES-256-GCM with AAD of scope, version and name (`rust/crates/baukit-credential-vault/src/cipher.rs`) | `rust/crates/baukit-credential-vault/src/tests.rs::wrong_associated_data_is_rejected`, `::tampered_ciphertext_nonce_and_empty_input_are_rejected` | covered |
| B-13 | inbound webhooks | S, T | Forged or replayed webhook body | HMAC signature with timestamp (`rust/crates/baukit-core/src/webhook_signature.rs`) | `rust/crates/baukit-core/tests/webhook_signature_vectors.rs::verification_cases_match_the_expected_result` | covered |
| B-14 | suite hub to product | S, T | Forged suite event or ticket | peer signatures and tickets (`rust/crates/baukit-suite/src/platform/receiver.rs`, `rust/crates/baukit-suite/src/platform/tickets.rs`) | `rust/crates/baukit-suite/tests/platform_signatures.rs` | covered |
| B-15 | client to njs media edge | S, T | Forged, altered or expired media grant | Rust signer and njs verifier with fixed query shape, expiry cap, constant-time compare (`rust/crates/baukit-core/src/media_grant.rs`, `deploy/media-grants/njs/media-grant.js`) | `deploy/media-grants/njs/media-grant.test.mjs`, `rust/crates/baukit-core/src/media_grant.rs` vector tests | covered |
| B-16 | browser and native app to API | T, I | Sync write lands under another owner's revision counter | per-owner revisions (`rust/crates/baukit-sync/src/revision.rs`) | `rust/crates/baukit-sync/tests/postgres.rs::owners_have_independent_counters` | covered |
| B-17 | browser and native app to API | R | No proof that an erasure or export request was honoured | signed receipts (`rust/crates/baukit-data-subject/src/receipt.rs`) | `rust/crates/baukit-test/src/data_subject_conformance.rs` | covered |
| S-01 | browser to API | I | XSS reads access and refresh tokens from `localStorage` | none, tokens stored by `typescript/packages/auth-web/src/index.ts` | to write: no storage write contains a refresh token | gap, P2.2 |
| S-02 | browser to API | T, I | Clickjacking, MIME sniffing, no CSP to contain XSS | none in `baukit-http`, templates or chart | to write: header assertions in the fixture gate | gap, P2.1 |
| S-03 | supply chain | T, E | Retagged GitHub Action runs with release permissions | none, actions pinned by tag | to write: zizmor and actionlint gate step | gap, P2.5 |
| S-04 | supply chain | T, I | Leaked secret or vulnerable JS dependency merges unnoticed | `cargo deny` for `rust/` only | to write: gitleaks and osv-scanner gate step | gap, P2.5 |
| S-05 | API to logs | I | `IssuedApiToken` plaintext printed through derived Debug (`rust/crates/baukit-auth/src/api_token.rs`) | none | to write: Debug output of every secret-holding type is redacted | gap, P2.3 |
| S-06 | IdP to API | S | Clerk token minted for another app without `azp` is accepted (`rust/crates/baukit-auth/src/verifier.rs`) | none, missing `azp` accepted by default | to write: negative test per token profile | gap, P2.4 |
| S-07 | API to third parties | I, E | Products build raw `reqwest` clients that skip the egress policy | `GuardedClient` exists, use is not enforced | to write: clippy `disallowed-methods` in template `clippy.toml` | gap, P2.9 |
| S-08 | browser to API | I, E | Id-only lookups let user B reach user A's rows | owner scoping by convention only | to write: `baukit-test` cross-tenant helper per OpenAPI route | gap, P2.8 |
| S-09 | all parsers | D, T | Panics or bypasses in decoders of untrusted bytes | none | to write: `rust/fuzz` targets and proptest properties | gap, P2.6, P2.7 |
| S-10 | all | all | New boundaries ship without a threat review | this file and the `baukit-threat-model` skill | product `docs/threat-model.md` checked in review | gap, P2.10, P2.11 |
| S-11 | IdP to API | S | Products lack negative auth tests | `baukit_test::jwt` fixtures only | to write: `baukit-test` auth negative suite (expired, iss, aud, alg none, HS256 confusion, unknown kid, tampered payload) | gap, P2.8 |

## Re-run when

- a crate, package or template adds an entry point, store, or outbound call;
- `baukit-auth`, `@baukit/auth-*`, the MCP security config or the media-grant format changes;
- a gap row closes (set status, cite the test) or a product reports a finding against baukit;
- before each minor release.
