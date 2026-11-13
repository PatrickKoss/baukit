# Changelog

## [Unreleased]

## [0.25.0] - 2026-11-13

## [0.24.0] - 2026-11-12

## [0.23.0] - 2026-11-12

## [0.22.0] - 2026-11-11

## [0.21.0] - 2026-11-10

## [0.20.0] - 2026-11-10

- Security: the link queries select by owner in SQL (`WHERE id=$1 AND user_id=$2`). `SuiteLinkStore::link_for_update`, `revoke_in_transaction`, `record_delivery`, `complete_delivery`, `record_received_in_transaction` and `replay_retry_after` now take the owner id before the link id, and `PostgresSuiteEventOutbox` scopes the replay cooldown read and write by owner. A foreign link id returns `NotFound`. Before, these loaded or updated by id alone and the services compared the owner in Rust. Implementors of the port must add the parameter. `find_link` stays unscoped for peer-authenticated ingest and background delivery.

## [0.19.2] - 2026-11-08

## [0.19.1] - 2026-11-08

## [0.19.0] - 2026-11-07

- `compose::PostgresSuite::builder(pool, settings, limiter)` replaces the `SuiteComposition` copied into product bins (P1.10). It takes `SuiteSettings` (`new` parses the catalog JSON, `with_catalog` takes a parsed one) plus the identity, replay, applier and erasure owner lookup ports, and exposes `context`, `outbox`, `store`, `delivery`, `erasure`, `module()`, `module_with(applier)`, `api()`, `notifier()`, `erasure_notifier(links)`, `router(api)` and, with `jobs`, `worker(store)`, `worker_with(store, delivery, config)` and `default_worker_config()`.
- Security: `PostgresSuite::router` limits `GET /suite/links/callback`, `POST /suite/links/exchange` and `POST /suite/links/{id}/revoke` per client IP (`SuitePublicRateLimit`, default 60 per minute, burst 10, fail closed). Before, a product that merged the suite routes outside `baukit_ratelimit::layers` left them unlimited.
- Breaking: `adapters::http::router` is gone and `adapters::http::public_router` is private. Mount the suite with `PostgresSuite::router(api)` or `adapters::public_limit::limited_router(state, store, limit)`, so callback, exchange and revoke cannot be mounted without the per-IP bucket. `inbound_router` and `user_router` stay public.
- `LinkCompleteRequest` and `LinkCallbackQuery` redact the authorization code in `Debug` output.
- `PostgresSuiteLinkStore::find_link_for_owner` scopes by owner in SQL and runs failure accounting only for the owner's link; a foreign id returns `None` without a write.

## [0.18.3] - 2026-11-07

## [0.18.2] - 2026-11-06

- Breaking: `OrganizationSummary` gains the required `sso_connection_active` and `directory_sync_active`, read from the hub summary's `ssoConnectionActive` and `directorySyncActive` (Schlüsselbund C34.F, BK29). Struct literals must set both, and a summary without them is `HubError::InvalidResponse`.

## [0.18.1] - 2026-11-05

## [0.18.0] - 2026-11-05

- Breaking: `HubClientConfig` gains `token`, a hub `sbsvc_` platform service token sent as the bearer with no token request, no refresh and no retry after a 401, and `token_url` becomes optional (BK21). Validation requires exactly one of the static token and client credentials.

- Breaking: organizations are plan kit subjects (BK25). `require_feature(plan, subject, feature)` takes `impl Into<FeatureSubject>` and returns `FeatureGrant`. `Some(suite_subject)` and `None` still work as account subjects. `FeatureSubject::Member` checks the organization's grant once the hub confirms the membership, and `FeatureSubject::Organization` checks an organization-owned API token or worker with no suite subject. `FeatureSubject` is an Axum extractor that reads the `PlanOrganization` extension, the `X-Organization-Id` header and `PlanSubject`. `HubAccountPlan::with_organizations(grants)` enables it, and grants and memberships stay cached for `ORGANIZATION_CACHE_TTL_SECONDS` (60). `AccountPlan` gains `organization_refresh`, `organization_current` and `is_member`, which default to `organization_not_accessible`. `PlanError` gains `OrganizationNotAccessible` (403 `organization_not_accessible`) and `OrganizationIdInvalid` (400 `organization_id_invalid`). `AccountEntitlement` gains `organization`, which the account body omits. `GET /me/entitlement` with `X-Organization-Id` serves the organization's plan to a member.

## [0.17.1] - 2026-11-03

## [0.17.0] - 2026-11-02

## [0.16.0] - 2026-11-02

## [0.15.0] - 2026-11-02

- `catalog_feature_keys`, `feature_key_drift` and `assert_feature_keys_match` count keys from a plan's `organizationProducts` as well as `includedProducts`. A missing or `null` list grants nothing. The recorded catalog in `fixtures/plans/v1/` is the hub's at Schlüsselbund C23, which grants `hebkit.trainer` only to organizations, so a product that receives an organization-only key must now declare it.

## [0.14.1] - 2026-11-01

## [0.14.0] - 2026-11-01

- Breaking: `TicketClaims` gains `billing: Option<BillingClaim>`, the optional Schlüsselbund C18 claim with `BillingChannel` (`web`, `app_store`, `play_store`, `family`, `organization`, `promo`), `sold_by` for store plans and `manage_url` for web plans. The verifier used to reject the claim as unknown; it now accepts a valid one and rejects `null`, unknown fields, `soldBy` outside the stores and `manageUrl` outside `web` or without an HTTP(S) host. `TicketClaims` is not `#[non_exhaustive]`, so struct literals need `billing: None`. The train takes a minor bump.
- `platform` adds the plan kit: the `AccountPlan` port with `StandaloneAccountPlan` and `HubAccountPlan`, `PlatformTicketSource` and `VerifiedTicketSource`, `require_feature`, `PlanError` with the codes `suite_identity_missing`, `entitlements_unavailable`, `credits_unavailable` and `feature_not_entitled`, `account_plan_router` for `GET /me/entitlement` and `GET /me/credits` with `PlanSubject` and `AccountPlanOpenApi`, where the entitlement route always refreshes so it carries the `billing` claim that `PlatformTicketSource` requests with `includeBilling: true` (Schlüsselbund C27), and `feature_key_drift` and `assert_feature_keys_match` against the recorded hub catalog in `fixtures/plans/v1/`. `CreditBalance`, `CreditGrant` and `CreditPack` derive `ToSchema`.
- Breaking: `CreditBalance` reads the bucketed balance of Schlüsselbund C21: `suite_subject`, `wallet`, `revision`, `total`, `grant` with `available` and `expires_at: Option<i64>`, and `packs` with `available` and `expires_at`. `available`, `pools`, `CreditPool`, `grant.granted`, `grant.used` and `packs[].remaining` are gone, and a balance in the old shape fails as `HubError::InvalidResponse`. Products read `total` instead of `available` and `packs[].available` instead of `remaining`, and need the hub at C21 or later. The train takes a minor bump.

## [0.13.0] - 2026-10-31

- Breaking: `OrganizationSummary` gains `name: Option<String>`, the organization's display name. The struct is not `#[non_exhaustive]`, so struct literals and exhaustive patterns outside the crate need the new field, and the train takes a minor bump. Decoding still accepts a summary without `name` from a hub before Schlüsselbund C16.
- Add `HubOrganizations::organizations_for_subject` for `GET /platform/v1/accounts/{suiteSubject}/organizations`, returning `SubjectOrganizationPage` with `SubjectOrganization` items and `next_cursor`. It needs Schlüsselbund C16.
- `HubClient` path segments now percent-encode every byte except ASCII alphanumerics and `*-._`, so `|` in a suite subject goes out as `%7C`. The credit balance path changes accordingly; the hub decodes both forms.
- `members` docs cover membership products, where every active member counts.

## [0.12.2] - 2026-10-31

- Add `HubClient::get_json_with_query` for paged `/platform/v1` routes. It form-encodes query names and values and shares `get_json`'s cached token, single 401 retry and error mapping. `get_json` is unchanged.
- Add `HubOrganizations::members` for `GET /platform/v1/organizations/{id}/members`, returning `OrganizationMemberPage` with `OrganizationMember` items and `next_cursor`. Products no longer need their own client-credentials token for the seat directory.

## [0.12.1] - 2026-10-31

## [0.12.0] - 2026-10-30

- Add the `organization.updated` and `credits.changed` platform events from the hub contract. Breaking: `PlatformEventKind` gains `OrganizationUpdated` and `CreditsChanged`, so exhaustive matches need new arms, and handlers now receive these events instead of having them recorded as unknown. The decoder checks the envelope `userId` against each type's natural key, which is `organizationId` for organization updates.
- Add `HubClient` and `HubClientConfig` for `/platform/v1` calls with a cached client-credentials token. The client refreshes a token 30 seconds before expiry, or at half its lifetime when that is shorter, and retries once after a 401.
- Add the `Credits` port with reserve, settle, release and balance, `ZeroCostCredits` for standalone mode, `HubCredits` for the hub, `CreditsError` for the hub credit error codes and `PlatformConfig::credits`.
- Add the `OrganizationDirectory` port and `HubOrganizations` for `GET /platform/v1/organizations/{id}`.

## [0.11.3] - 2026-10-28

## [0.11.2] - 2026-10-28

## [0.11.1] - 2026-10-24

## [0.11.0] - 2026-10-22

- Clarify that handlers overriding `prepare_at` must delegate `prepare` to it.

- Keep entitlement events retryable when a custom inbox skips preparation. Pass the receiver clock to `PlatformEventHandler::prepare_at` for deterministic free-cache deadlines.
- Document that erasure removes revision cursors and products must filter late events for erased subjects.

- Add per-peer hosted `authorize_url` overrides with config and registry validation. The full path and suite protocol query parameters are preserved.
- Breaking: `PeerUrls`, `PeerRegistrySettings` and `SuiteConfig` are non-exhaustive. Replace external struct literals with `PeerUrls::new`, `PeerRegistrySettings::default` and their builders, or mutate defaults. `ActivePeer` gains `authorize_url: Option<url::Url>`, parsed during registry validation. This requires a minor release.
- Add the read-only `SuiteLinksRead` service port, implemented by `SuiteLinkService`.
- Apply job migration 0004 in PostgreSQL fixtures for the store's new key columns.
- Add read-through entitlement refresh with per-subject single-flight, explicit free fallback, transactional cache erasure and batched receipt retention. Configure the hub source and event prefix, prepare pulls before receipt transactions, and keep cached ticket verification available during JWKS refresh.
- Revise the unreleased platform API: public event context constructors, `PlatformEventDecoder`, `PlatformEventHandler::prepare`, cache revision and erasure methods, a required source for hub reads, `EntitlementSourceError`, and detailed configuration errors from `ticket_verifier`.
- Add the optional `platform` feature with typed hub events, a signed receiver, transactional PostgreSQL receipts, entitlement caches, standalone reads and a cached Ed25519 ticket verifier.

## [0.10.32] - 2026-10-21

## [0.10.31] - 2026-10-20

- Release the coordinated baukit 0.10.31 train.

## [0.10.30] - 2026-10-20

- Release the coordinated baukit 0.10.30 train.

## [0.10.29] - 2026-10-15

## [0.10.28] - 2026-10-14

## [0.10.27] - 2026-10-14

## [0.10.26] - 2026-10-13

## [0.10.25] - 2026-10-13

## [0.10.24] - 2026-10-13

## [0.10.23] - 2026-10-12

## [0.10.22] - 2026-10-12

## [0.10.21] - 2026-10-12

## [0.10.20] - 2026-10-12

- Check erasure admission before notifying peers. Old-key replays after re-onboarding, fenced sessions, invalid keys and foreign keys leave the current profile's suite links intact.

## [0.10.19] - 2026-10-11

## [0.10.18] - 2026-10-11

## [0.10.17] - 2026-10-11

## [0.10.16] - 2026-10-11

## [0.10.15] - 2026-10-11

## [0.10.14] - 2026-10-11

## [0.10.13] - 2026-10-10

- Export the sorted suite error codes from the same definition as the Rust mappings and OpenAPI enum.
- The OpenAPI `lastFailureCode` enum on suite link delivery status is now `SuiteErrorCode` with every value in `SUITE_ERROR_CODES`, replacing the ten-value `SuiteDeliveryFailureCode`. Generated clients with an exhaustive map over it need entries for the added codes.

## [0.10.12] - 2026-10-10

## [0.10.11] - 2026-10-10

## [0.10.10] - 2026-10-10

- Export `SUITE_JOB_TYPES` for products to pass to `PostgresJobStore::retain_kinds`.
  This keeps suite delivery and revoke history until suite cleanup removes it
  after 30 days, even when generic job cleanup uses a shorter cutoff.

## [0.10.9] - 2026-10-10

## [0.10.8] - 2026-10-09

## [0.10.7] - 2026-10-09

- Breaking: Add `SuiteStoreError::QuotaExceeded { retry_after_seconds }` for applier quotas that reset. Exhaustive matches need a new arm. Inbound returns 429 `suite_quota_exceeded` with `Retry-After` of at least one second. Permanent owner caps keep returning `Capped`.

## [0.10.6] - 2026-10-09

## [0.10.5] - 2026-10-08

## [0.10.4] - 2026-10-08

## [0.10.3] - 2026-10-08

- Keep hourly cleanup alive after storage errors. Log each failure and count it in `suite_cleanup_failures_total`.
- Guard cleanup across replicas and lock affected owners before links and jobs. Add `003_suite_lock_order.sql` to record failed jobs without locking links, then account for failures before health reads and writes.
- Add a default `SuiteEventApplier::lock_owner` hook before ingest locks a link.
- Return 422 `suite_payload_invalid` for typed payload rejections. Export the field validators and keep existing applier signatures.
- Accept empty boolean settings as false.
- Cover HTTP error codes, rate-limit keys, delivery revocation, link isolation, failed exchanges, replay, URLs and wire contracts in protocol tests.
- Use real catalog types in the event-id vectors. Products that copy the vectors should refresh them.

## [0.10.2] - 2026-10-08

## [0.10.1] - 2026-10-08

- Keep the default build free of database, Tokio and HTTP dependencies. Gate SQLx ports and stores behind `postgres`, services behind `runtime`, and the guarded peer client behind `delivery`.
- Check each feature build with Clippy and test a domain consumer without features.

## [0.10.0] - 2026-10-08

- Look up metadata for the own app and inactive peers by id. Share peer validation vectors with the client.
- Document how existing suite migrations and pending release packages adopt the crate.
- Add product-neutral peer linking, PKCE, sealed per-link secrets and catalog-driven payload validation.
- Add transactional outbox and inbox stores, signed delivery and revocation, rate limits, circuit accounting, replay, HTTP routes, job handling and hourly cleanup.
- Send bounded peer revokes before account erasure, then delete suite rows in the product transaction.
- Lock the product owner before suite cleanup so concurrent domain writes cannot leave delivery jobs behind.
- Serialize replay, connection-test and disconnect jobs with erasure before locking links.
- Add a domain transaction owner lock and reject unfenced enqueues during erasure without aborting the product transaction.
- Validate shared protocol fixtures and payload vectors against unit tests and PostgreSQL router tests.
