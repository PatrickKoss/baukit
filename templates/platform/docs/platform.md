# Platform receiver

This product was generated with `baukit new --backend --platform`.
`capabilities.platform = true` records the capability in `baukit.toml`.
`[platform].migration` records its product-owned SQL migration for doctor.
Those are generation settings. Deployment-specific values belong in backend
runtime configuration and secret storage, so each deployment can choose its hub.

## Runtime configuration

For local development, run `make setup`, `make db-up` and `make migrate` before
`make run`. The root `.env.example` has the local PostgreSQL URL and standalone
settings. No hub is required.

The backend embeds `PlatformConfig` as `ProductConfig.platform` and the ticket
source settings as `ProductConfig.platform_hub`. All environment keys start with
`{{ context.app_env }}__`. See the generated root `.env.example`.

| Suffix | Meaning |
| --- | --- |
| `PLATFORM__HUB_ISSUER` | Exact ticket issuer. Omit for standalone mode. |
| `PLATFORM__JWKS_URL` | Trusted public-key endpoint. Required in hub mode. |
| `PLATFORM__PRODUCT_ID` | Hub product audience, usually `{{ context.app_crate }}`. |
| `PLATFORM__SOURCE_APP` | Expected envelope source and signature source header. |
| `PLATFORM__EVENT_TYPE_PREFIX` | Optional distinct event prefix. |
| `PLATFORM__RECEIVER_SECRET` | Current HMAC secret. |
| `PLATFORM__PREVIOUS_RECEIVER_SECRET` | Previous secret during rotation. |
| `PLATFORM__STANDALONE_DEFAULT__TIER` | Standalone tier, initially `free`. |
| `PLATFORM__STANDALONE_DEFAULT__FEATURES` | JSON array of standalone feature keys, initially `[]`. |
| `PLATFORM_HUB__TICKET_URL` | Full hub ticket endpoint URL. |
| `PLATFORM_HUB__SERVICE_TOKEN` | Server-side bearer token with entitlement-read scope. |
| `PLATFORM_HUB__ALLOW_LOCAL_HTTP` | Allow loopback HTTP for tests and local development, initially `false`. |

With no hub settings, `platform::compose` exposes an `Entitlements` service that
returns the standalone defaults. It mounts no inbound route. PostgreSQL still
stores product data. Partial hub configuration fails validation.
In hub mode, the API merges `POST /platform/inbound` outside user authentication.
HMAC authenticates the raw bytes. The OpenAPI spec includes the route in both
modes. `Extension<Entitlements>` is available to product HTTP handlers. Read with
`entitlements.current(subject, chrono::Utc::now().timestamp()).await?`,
`has_entitlement(subject, feature, now)` or `has_tier(subject, tier, now)`.
Product code defines feature and tier meanings.

The API adapter `HubTicketSource` POSTs `{suiteSubject, productId}` to the configured
ticket URL with the service token, then verifies the returned `{ticket}` using
hub JWKS. It binds issuer, audience and subject before caching. It allows public
HTTPS by default, rejects redirects and limits responses to 64 KiB. Adapt its
address policy for an explicitly allowed private hub. The token is an injected
credential. Rotate it before expiry, or replace the source through
`platform::compose_with(pool, config, source, events)` with an adapter that
refreshes client-credentials tokens, such as one built on
`baukit_suite::platform::HubClient`. Never put this token in a browser or app.

`HubClient` caches its client-credentials token and refreshes it before expiry
or once after a 401. `PlatformConfig::credits` returns `ZeroCostCredits` in
standalone mode and `HubCredits` in hub mode for metered actions, and
`HubOrganizations` reads organization summaries after `organization.updated`.

Run migrations before starting the API. On cache miss or expiry, reads pull
current state. Entitlement notifications also pull before their receipt
transaction and store the snapshot in that transaction. Read failures return
errors instead of expired paid grants. Refresh active subjects daily and pull
current state when enabling a receiver, because the hub does not backfill.

## Product handler

Implement the product's `PlatformEvents` port in the ports crate, and pass it to
`platform::compose_with`. The default `LoggingPlatformEvents` logs event types.
It returns a retryable error for erasure requests until the product implements
its erasure policy. Entitlement events use `EntitlementEventHandler`; other
known events reach the product handler with `EventContext`. Use
`context.postgres()` for writes that must commit with the receipt. Return
`PlatformError::Unavailable` for a retry and `Invalid` for permanent rejection.

## Erasure and late events

{% if context.auth_enabled %}The generated `PostgresProfileErasure` calls
`PostgresEntitlementCache::forget_in_transaction(subject, connection)` in the
same transaction as profile deletion. The generated test covers commit and
rollback. Hub erasure handling must call the product erasure service and report
completion to the hub after product erasure finishes.
{% else %}This flavor has no account-erasure flow. Call
`PostgresEntitlementCache::forget_in_transaction(subject, connection)` in a future
product erasure transaction, or `EntitlementCache::forget(subject)` outside one.
{% endif %}
Both methods delete snapshots and revision cursors for every product of the
subject. A late `entitlements.changed` can pull and store a row again. Add a
subject-existence filter outside `EntitlementEventHandler`, before its
`prepare_at` call. Acknowledge events for erased or unknown subjects without
refreshing. Filtering inside `PlatformEvents::handle` is too late because
entitlement events bypass that handler. Also check subject existence before
read-through calls. The product owns its suite-subject mapping and this filter.

## Receipt retention

{% if context.worker %}The durable worker registers `PlatformJobHandler` alongside the demo job handler.
At startup, every replica calls `enqueue_initial(now)`. A daily `FixedUtcInterval`
slot supplies the idempotency key, so replicas and restarts share one job per
UTC day. After purging bounded batches, the handler commits the next slot and
current job completion in one transaction. Delayed jobs skip missed days.
Monitor failed `platform.receipts.purge` jobs and retry them through the job
operations API after fixing the failure. The worker must be running for retention
to advance.

`RECEIPT_RETENTION_DAYS` lives in the worker crate's `platform` module. Increase
it for a longer replay window; never lower it below 30 days.
{% else %}This flavor has no durable worker. Schedule this command daily with the
deployment's scheduler, using the same runtime database settings:

```sh
cargo run --manifest-path backend/Cargo.toml -p {{ context.app_name }}-bin --bin platform-purge
```

`platform::purge_receipts(pool)` is the equivalent hook for a product maintenance
job. It deletes bounded batches until none remain. `RECEIPT_RETENTION_DAYS` lives
in the binary crate's `platform` module. Increase it for a longer replay window;
never lower it below 30 days. The generated Dockerfile can include the command
by adding `platform-purge` to `backend.image.binaries` and running
`baukit generate dockerfile`.
{% endif %}
Pruning receipts permits replay, so external effects must tolerate it.

## Deploy and client checks

Generated Helm values use the chart's `platform.receiverSecretRef` keys `name`,
`key` and `previousKey`. The generated defaults leave `name` and `previousKey`
empty, so a standalone deployment needs no receiver Secret. The commented hub example in
`deploy/values.yaml` supplies the issuer, JWKS URL, product, source and ticket URL
through `config.overrides`. In hub mode, create the receiver Secret through the
secret manager. Keep `previousKey` empty unless rotating. Put
`PLATFORM_HUB__SERVICE_TOKEN` in the product runtime Secret. Migration and worker
commands load the same runtime configuration. The hub example gives them an
explicit reference to the current receiver secret so validation also succeeds
in those processes. Allow network egress to the hub and expose `/platform/inbound`
through the API ingress.

Web and mobile templates have no entitlement-check location. When adding one,
install `@baukit/suite-client` and use its offline verifier with trusted public
keys. Fetch and refresh those public keys separately:

```ts
import { verifyTicket } from '@baukit/suite-client/entitlements';

const claims = verifyTicket(ticket, trustedJwks, {
  issuer: configuredHubIssuer,
  audience: productId,
  now: Math.floor(Date.now() / 1000),
});
if (claims.sub !== currentSuiteSubject) throw new Error('ticket subject mismatch');
```
