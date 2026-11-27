# Fail-closed answers, stale grace and new error variants

0.32.0 makes the crates that depend on another service say "unavailable, retry after N seconds"
instead of "limited" or a bare error, and keeps known-good data around longer while that service is
down. Most of the work on upgrade is adding match arms for new variants.

## Rate limiting

In `closed` fail mode, a rate-limit store error now answers `503` with `Retry-After: 1` instead of
`429` with the whole quota period. That holds for route groups and for the browser-session limiter.
`open` mode, the default, is unchanged. Tests that assert `429` for a failing store in closed mode must
expect `503`.

`baukit_auth::browser_session::RateLimitOutcome` gains `Unavailable { retry_after }`. A browser-session limiter you
implement yourself returns it when its store fails closed. A `match` on the outcome must handle it.

`RedisRateLimitStore::connect_if_enabled` takes a startup budget. See
[startup-budget-and-postgres-timeouts.md](startup-budget-and-postgres-timeouts.md).

## Auth

The default JWKS stale grace goes from 10 min to 1 h, so verification with known key ids survives a
longer IdP outage on the last good key set. Unknown key ids never use a stale set. To keep 10 min:

```rust
let config = config.with_jwks_stale_grace(Duration::from_secs(600));
```

`with_jwks_stale_window` sets the same value and rejects zero.

OIDC discovery retries within a startup budget. Two new variants report the end of the budget:
`VerificationError::DiscoveryBudgetExhausted` from `OidcVerifier::discover` and
`IdpDiscoveryError::BudgetExhausted` from `BrowserSessionState::discover`. Issuer mismatches and invalid
documents still fail at once. `OidcConfig::with_jwks_uri` sets a static JWKS URL and skips discovery;
an invalid URL fails with the new `OidcConfigError::InvalidJwksUri`.

The browser-session `idp_unavailable` answer (503) now sends `Retry-After`, and an open `idp` circuit
maps to it and keeps the session.

## MCP

A `503 authentication_policy_unavailable` answer carries `Retry-After: 5`. An open
`mcp-introspection` circuit returns that answer without calling Keycloak. No code change.

## Suite entitlements

`Entitlements::current`, `has_entitlement` and `has_tier` return `EntitlementError` instead of
`PlatformError`:

| Variant | Meaning |
| --- | --- |
| `Unavailable { retry_after }` | the hub failed and no snapshot is inside its grace window |
| `Platform(PlatformError)` | the entitlement cache failed |

`From<EntitlementError> for ApiError` turns `Unavailable` into `503` with `Retry-After`, so a handler
that uses `?` keeps working once its error type converts. While the hub fails, `Entitlements::read`
serves the last snapshot until its `grace_until` and sets `stale: true` on the `EntitlementRead`. A custom
`EntitlementCache` must implement `retained(subject, product_id)`, which returns the stored snapshot even
after `valid_until`. `EntitlementSourceError::CoolingDown` carries the source's own delay.

## Suite hub client

`HubClient::post_json` is gone. Build a `HubRequest` and call `send_json`:

| 0.31.x | 0.32.0 |
| --- | --- |
| `hub.post_json(segments, &body)` | `hub.send_json(HubRequest::post(segments, &body)?)` |
| a POST that is safe to replay | `HubRequest::idempotent(segments, &body)?` or `.with_idempotency_key(key)?` |

GETs, `idempotent` requests and requests with an `Idempotency-Key` replay once after a `401`.
`HubRequest::post` never does. An open egress breaker maps to `HubError::CoolingDown { retry_after }`.

## Suite delivery probes

Delivery failures now count per attempt in the store, against `SUITE_CIRCUIT_FAILURES`, instead of per
terminal job in a SQL trigger. A disabled link gets a probe delivery after 1 hour, then after a delay
that doubles up to 24 hours. An open egress breaker defers the delivery and does not count.

Apply `SUITE_DELIVERY_PROBES_MIGRATION_SQL` (`006_suite_delivery_probes.sql`) after `005`. It is not
expand-only. It drops the `suite_delivery_failed` trigger, the accounting functions and the
`suite_failed_delivery_jobs` table that a 0.31.x binary queries, and adds `next_probe_at` and
`failed_probes` to `suite_links`. Stop the 0.31.x processes that run suite deliveries, apply the
migration, then start 0.32.0. Delivery jobs now carry a concurrency key, so apply `baukit-jobs` `0006`
first, as [job-deferrals-and-concurrency-keys.md](job-deferrals-and-concurrency-keys.md) says.

For custom suite stores and direct calls to the domain functions:

- `SuiteLinkStore` gains `claim_probe(owner_id, link_id, now)`.
- `SuiteLink` gains `next_probe_at: Option<Timestamp>` and `failed_probes: u32`.
- `delivery_preflight(link, now)` and `after_delivery(state, action, now)` take `now`.
- `default_worker_config` sets `max_per_key` to `SUITE_DELIVERY_MAX_PER_KEY` (1), and
  `suite_delivery_concurrency_key(peer_app)` builds the key.

## Erasure

`IdentityDeletionError` gains `Unavailable { retry_after }`. An open `keycloak-admin` circuit or a
saturated client maps to it, and the erasure worker defers the job with `JobError::deferred` instead of
consuming an attempt. A custom `IdentityAccountDeleter` can return it, and a `match` on the error must
handle it.

The inline erasure path no longer holds `SELECT ... FOR UPDATE` across the identity provider call. It
reserves the job through `run_after` (the inline timeout plus 5 s), calls the provider, then re-checks
the job under a lock. No code change.

## Push

`PushSender::send` fails with `PushSendError` instead of `PushError`. It replaces `PushError::Partial`
and `PushError::completed_outcomes`:

| Field | Holds |
| --- | --- |
| `error` | the `PushError` of the failing chunk |
| `outcomes` | results of the chunks Expo already accepted, including tickets that still need a receipt poll |
| `unsent` | the failing chunk and every chunk after it |

Record `outcomes`, then retry only `unsent`, so no notification goes out twice.
`PushSendError::is_retryable()` and `retry_after()` forward to `error`. A custom `PushSender` wraps a
failure before the first chunk with `PushSendError::before_any(error, unsent)`.

## Test fixtures

`RedisSentinelTestContainer::stop_master` fails with the new `RedisTestError::Fault` instead of
`RedisTestError::Container`. Update tests that match on it.
