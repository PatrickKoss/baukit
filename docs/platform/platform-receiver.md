# Platform receiver and entitlement tickets

`baukit-suite/platform` receives service-level hub notifications independently of
peer links. It adds no account, plan, billing or product-specific access rules.
The mirrored v1 contract retains its original event type names and `sourceApp`.

## Adopt the receiver

Enable `features = ["platform", "postgres"]` on `baukit-suite`. Copy
`rust/crates/baukit-suite/migrations/004_platform.sql` into the product's next
forward migration. Its SQL is also exported as `platform::PLATFORM_MIGRATION_SQL`.
Run it through the product's migration command before starting the API. It has no
jobs or peer-link migration dependency. The library never runs migrations.

Embed `platform::PlatformConfig` as `platform` in the product settings. Call its
`Validate::validate` implementation from the product settings validator. Construct
`PostgresPlatformInbox::new(pool.clone())` and
`PostgresEntitlementCache::new(pool.clone())` with the same database.

Implement `PlatformEventHandler::handle(&self, event: &PlatformEvent, context:
&mut EventContext<'_>) -> Result<(), PlatformError>`. Match `event.kind()` to get
typed payloads for entitlement, account, erasure, preference, onboarding, family,
organization and seat events. Use `context.postgres()` for product writes that
must commit with receipt of the event. Return `PlatformError::Unavailable` when
the hub should retry. Return `PlatformError::Invalid` for permanent rejection.
External effects must tolerate retries, including cancellation after a successful
external call. Erasure remains a product handler responsibility. Delete the
subject's entitlement cache rows within that erasure transaction, and report the
completed erasure through the hub API after product erasure finishes.

Call `settings.platform.receiver(inbox, handler)?`. If it returns `Some(receiver)`,
merge `platform::router(receiver)` into the public router at its root. It exposes
`POST /platform/inbound`. Do not put it behind user-token authentication. HMAC
verification authenticates the raw request bytes with the current or previous
secret. Requests must have one of each signature header, a canonical Unix-second
timestamp within 300 seconds, and `Content-Type: application/json`.
`PlatformReceiver::with_body_limit(bytes)` changes the default 16 KiB limit.
`with_clock(Arc<dyn PlatformClock>)` supplies a clock for tests.

The PostgreSQL inbox inserts its event receipt and runs the handler in one
transaction. Concurrent copies of an event wait for that transaction, then return
success without calling the handler again. Handler failure or cancellation rolls
back both writes and receipt. Reusing an event ID with different raw bytes returns
409. Retain receipts for at least the hub's delivery and operational replay window;
pruning them allows replay. Receipt rows retain only the event ID and body hash.
The memory inbox is a test adapter and loses receipts on restart.

| Response | Hub behavior |
| --- | --- |
| 204 | Applied, duplicate or unknown event type. Stop delivery. |
| 401 | Missing, malformed, stale or invalid signature. Mark receiver for attention. |
| 409 | Event ID reused with changed body. Permanent rejection. |
| 413, 415, 422 | Body too large, wrong content type, or invalid known event. Permanent rejection. |
| 503 | Handler or storage unavailable. Retry. |

Unknown types are acknowledged and deduplicated without dispatch. Known payloads
reject unknown fields, nulls, unsupported schemas and mismatched subjects through
the mirrored catalog. Events may be older than seven days because delivery retries
can span longer outages. Their signed attempt timestamp must still be fresh.

## Refresh and read entitlements

Implement `EntitlementSource::pull(subject, product_id)` using the product's
service credentials and the hub entitlement or ticket endpoint. Return a bound
`EntitlementSnapshot` with the hub revision and an exclusive `valid_until`
deadline. A verified `TicketClaims` converts into a snapshot with `.into()`.
No service credentials belong in a web or mobile client.

Wrap the product handler in `EntitlementEventHandler::new(product_id, cache,
source, product_handler)`. Entitlement events pull current state and store it
through the receipt transaction; other events go to the product handler. A pull
with the wrong subject or product, or a revision below the notification, fails
and the hub retries. Delayed events cannot overwrite a newer cache revision.
Equal revisions only replace snapshots when their deadline increases. Refresh
once a day as well as on events, and pull current state after enabling a receiver
because the hub does not backfill notifications emitted while it was disabled.

`settings.platform.entitlements(cache)?` returns the shared `Entitlements` read
API. `current(subject, now)` returns an unexpired snapshot or `None`.
`has_entitlement(subject, feature, now)` checks a feature key;
`has_tier(subject, tier, now)` compares the tier exactly. Neither method infers a
tier hierarchy. Missing and expired snapshots grant no features. Storage failures
remain errors. Expired rows retain revision cursors so older grants cannot return.

Without `hub_issuer`, the same API returns `standalone_default` for every subject
and makes no hub calls. Set its tier and feature keys to the product's desired
self-hosted policy. The library default is `free` with no feature keys.

## Verify tickets

Construct `TicketVerifier` through `settings.platform.ticket_verifier(client)?`
with a `baukit_egress::GuardedClient`. Its address policy controls public, loopback
or explicitly allowed private-network JWKS access. Redirects are rejected and
network deadlines and response size limits come from that client. The verifier
also rejects JWKS bodies above 64 KiB and sets above 64 keys.

`verify(ticket).await` checks Ed25519, key metadata, the exact issuer and product
audience, the claim schema and a seven-day maximum lifetime. `verify_at(ticket,
now)` accepts an explicit Unix-second clock. Expiry is exclusive with no allowance;
`iat` may be at most 60 seconds ahead. Verify that the returned `sub` is the
subject requesting access before storing or using the result.

Keys are cached by `kid` for 300 seconds. An unknown key triggers a refresh even
while the cache is fresh. Unknown-key refreshes have a shared five-second cooldown
and concurrent refreshes share a lock. Failed fetches also have a five-second
cooldown. An unavailable JWKS never extends the cache lifetime. The stable ticket
errors are `ticket_invalid`, `ticket_expired`, `ticket_wrong_audience` and
`ticket_keys_unavailable`. `verify_ticket(ticket, jwks, issuer, audience, now)`
provides the same checks offline with trusted keys. `TicketJwksSource` permits a
product-owned key source or a test double.

For web and React Native:

```ts
import { verifyTicket } from '@baukit/suite-client/entitlements';

const claims = verifyTicket(ticket, trustedJwks, {
  issuer: configuredHubIssuer,
  audience: productId,
  now: Math.floor(Date.now() / 1000),
});
if (claims.sub !== currentSuiteSubject) throw new Error('ticket subject mismatch');
```

The client verifier is synchronous and offline. Products fetch or embed trusted
public keys and own refresh and persistence. It uses `@noble/curves` 1.9.7, already
present in the workspace lockfile, for pure JavaScript Ed25519 and SHA-512. The
[upstream release](https://github.com/paulmillr/noble-curves/releases/tag/1.9.7)
predates this change by more than two weeks. It needs neither WebCrypto Ed25519 nor
Node globals, `atob`, `TextEncoder` or `TextDecoder`, so the same implementation can
run on Hermes. Both languages run the hub's positive and negative ticket fixtures.

## Configure and deploy

With product prefix `APP`, these settings load through `baukit-config`:

| Environment key | Meaning |
| --- | --- |
| `APP__PLATFORM__HUB_ISSUER` | Exact ticket issuer. Absent selects standalone mode. |
| `APP__PLATFORM__JWKS_URL` | Explicit hub public-key endpoint. Required in hub mode. |
| `APP__PLATFORM__PRODUCT_ID` | Product audience. Required in hub mode. |
| `APP__PLATFORM__RECEIVER_SECRET` | Current receiver HMAC secret, held as `Secret<String>`. |
| `APP__PLATFORM__PREVIOUS_RECEIVER_SECRET` | Optional previous HMAC secret during rotation. |
| `APP__PLATFORM__STANDALONE_DEFAULT__TIER` | Default standalone tier, initially `free`. |

Configure `standalone_default.features` as an array in the product settings file.
Partial hub settings fail validation. No secret value appears in settings debug
output. During rotation, install the previous secret alongside the current secret,
then remove it after in-flight attempts and the timestamp window have drained.

The Helm chart can mount an existing Kubernetes Secret only into the API:

```yaml
config:
  overrides:
    PLATFORM__HUB_ISSUER: https://hub.example
    PLATFORM__JWKS_URL: https://hub.example/keys.json
    PLATFORM__PRODUCT_ID: alpha
platform:
  receiverSecretRef:
    name: alpha-platform-receiver
    key: current
    previousKey: previous
```

`previousKey` is optional. Create that Secret through the deployment's secret
manager. The chart stores references only. Add network-policy egress to the hub
and expose `/platform/inbound` through the product ingress. Products may instead
use `config.existingSecretRefs` with fully prefixed environment keys.
