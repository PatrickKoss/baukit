# Platform receiver and entitlement tickets

`baukit-suite/platform` receives service-level hub notifications independently of
peer links. It adds no account, plan, billing or product-specific access rules.
Configure the expected `sourceApp` and event type prefix for the hub. The payload
rules match the mirrored v1 contract; Baukit does not choose a hub name.

## Generate the receiver

Run `baukit new my-product --backend --platform` to generate the receiver,
PostgreSQL migration, free standalone reads, verified hub ticket source, product
handler port and receipt retention. With `--worker`, daily fixed-slot jobs purge
receipts. Other flavors provide a `platform-purge` command for an external
scheduler. Add `--auth oidc` for the generated account erasure flow. The generated
`docs/platform.md` describes runtime settings and adoption work. See the [CLI guide](../../cli/README.md#platform-receiver).
Existing products can re-render with `new --into-existing`; review the conflicts
and copy the wiring into their own composition.

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
organization membership, seat, organization update and credit change events. Use `context.postgres()` for product writes that
must commit with receipt of the event. Return `PlatformError::Unavailable` when
the hub should retry. Return `PlatformError::Invalid` for permanent rejection.
External effects must tolerate retries, including cancellation after a successful
external call. Erasure remains a product handler responsibility. Call
`PostgresEntitlementCache::forget_in_transaction(subject, connection).await?` on
`context.postgres()` during that erasure transaction. Outside a transaction, use
`EntitlementCache::forget(subject).await?`. Both remove every product snapshot and
revision cursor for the subject. A late `entitlements.changed` can therefore pull
and store a fresh row for an erased subject. Check that the subject still exists
before forwarding to `EntitlementEventHandler::prepare_at`, and acknowledge events
for unknown or erased subjects without refreshing. A filter inside the wrapped
product handler runs too late because entitlement events bypass it. Apply the
same existence check before read-through entitlement calls. The product owns
this check and its suite-subject mapping. Report completed erasure through the hub
API after product erasure finishes.

Call `settings.platform.receiver(inbox, handler)?`. If it returns `Some(receiver)`,
merge `platform::router(receiver)` into the public router at its root. It exposes
`POST /platform/inbound`. Do not put it behind user-token authentication. HMAC
verification authenticates the raw request bytes with the current or previous
secret. Requests must have one of each signature header, a canonical Unix-second
timestamp within 300 seconds, and `Content-Type: application/json`.
`PlatformReceiver::with_body_limit(bytes)` changes the default 16 KiB limit.
`with_clock(Arc<dyn PlatformClock>)` supplies a clock for tests.

The inbox calls `PlatformEventHandler::prepare(event)` before opening a receipt
transaction. The receiver forwards its clock through
`PlatformEventHandler::prepare_at(event, now)`, which defaults to `prepare(event)`
for existing handlers. Use that hook when preparation needs time. Its optional
returned handler applies prepared data inside the transaction. Put network calls
in `prepare`; use `handle` for transactional writes. A custom inbox must follow the same order. Skipping preparation on an
entitlement event returns `Unavailable` and HTTP 503 so the hub can retry. An inbox
can construct `EventContext::detached()` or
`EventContext::with_postgres(&mut connection)`.
`PlatformOpenApi` describes the endpoint for inclusion in product OpenAPI docs.

The PostgreSQL inbox inserts its event receipt and runs the selected handler in
one transaction. Concurrent copies of an event wait for that transaction, then
return success without calling the handler again. Handler failure or cancellation rolls
back both writes and receipt. Reusing an event ID with different raw bytes returns
409. Retain receipts for at least 30 days to match the hub's delivery retention.
For a longer operational replay window, keep them longer. Schedule
`inbox.purge_received_before(Utc::now() - TimeDelta::days(30), 1000).await?`
in a maintenance job, repeating batches until it returns zero. The cutoff is
exclusive and the batch must be positive. An index on `received_at` supports the
cleanup. Pruning receipts allows replay. Receipt rows retain only the event ID,
body hash and receipt timestamp.
The memory inbox is a test adapter and loses receipts on restart.

| Response | Hub behavior |
| --- | --- |
| 204 | Applied, duplicate or unknown event type. Stop delivery. |
| 401 | Missing, malformed, stale or invalid signature. Mark receiver for attention. |
| 409 | Event ID reused with changed body. Permanent rejection. |
| 413, 415, 422 | Body too large, wrong content type, or invalid known event. Permanent rejection. |
| 503 | Handler or storage unavailable. Retry. |

Unknown types are acknowledged and deduplicated without dispatch. Known payloads
reject unknown fields, nulls, unsupported schemas and mismatched owners through
the mirrored catalog. The envelope `userId` must equal the type's natural key:
`suiteSubject` for every type except `organization.updated`, whose owner is
`organizationId`. Events may be older than seven days because delivery retries
can span longer outages. `occurredAt` cannot exceed the receiver clock by more
than `MAX_EVENT_FUTURE_SECONDS` (300 seconds). The signed attempt timestamp must
still be fresh. `PlatformEventDecoder::new(source_app, event_type_prefix)` also
provides these checks outside the router. Its `decode(body, now)` returns a typed
event; serializing that event preserves the original envelope. Event-kind serde
tags contain the suffix, such as `entitlements.changed`. The catalog is parsed
once per process.

## Refresh and read entitlements

Implement `EntitlementSource::pull(subject, product_id)` using the product's
service credentials and the hub entitlement or ticket endpoint. Return a bound
`EntitlementSnapshot` with the hub revision and an exclusive `valid_until`
deadline. A verified `TicketClaims` converts into a snapshot with `.into()`.
The return type is `Result<EntitlementSnapshot, EntitlementSourceError>`. For an
HTTP error response, return
`EntitlementSourceError::from_hub_response(status.as_u16(), error_code)`.
Only `404 platform_subject_not_found` and `403 account_inactive` map to
`NoEntitlements`. An explicitly absent snapshot also maps to `NoEntitlements`.
Other errors, including `product_unavailable`, `scope_missing`, `product_mismatch`
and an unrelated `404 not_found`, remain failures.
No service credentials belong in a web or mobile client.

Wrap the product handler in `EntitlementEventHandler::new(product_id, cache,
source, product_handler)`. Entitlement events pull current state before opening
the receipt transaction, then store it inside the transaction. Other events go to
the product handler. A pull with the wrong subject or product, or a revision
below the notification, fails and the hub retries. Invalid pull responses also return 503, since the signed
notification itself may be valid. Delayed events cannot overwrite a newer cache
revision.
Equal revisions only replace snapshots when their deadline increases. Refresh
once a day as well as on events, and pull current state after enabling a receiver
because the hub does not backfill notifications emitted while it was disabled.

`settings.platform.entitlements(cache, Some(source))?` returns the shared
`Entitlements` read API in hub mode. The direct constructor is
`Entitlements::hub(product_id, cache, source)`. `current(subject, now)` reads an
unexpired snapshot. On a miss or expiry, it pulls through the source, validates
the subject, product, revision and deadline, stores the result and reads it back.
Clones of the service share one in-flight pull per subject. Concurrent subjects
do not block each other. A newer revision arriving during the pull wins.
`has_entitlement(subject, feature, now)` checks a feature key;
`has_tier(subject, tier, now)` compares the tier exactly. Neither method infers a
tier hierarchy. When the source reports `NoEntitlements`, the API caches a
`free` tier with no features for `FREE_CACHE_TTL_SECONDS` (60 seconds), keeping
the previous revision cursor. A missing hub snapshot therefore grants the free
tier on first use. Other source or storage failures fail closed: the read returns
an error and never serves an expired paid grant or silently substitutes free.
Expired rows retain revision cursors so older grants cannot return.
`EntitlementCache::revision(subject, product_id)` reads that cursor without
returning an expired grant.

Without `hub_issuer`, the same API returns `standalone_default` for every subject
and makes no hub calls. Use `settings.platform.entitlements(cache, None)?`. Set
its tier and feature keys to the product's desired self-hosted policy. The library
default is `free` with no feature keys.

## Call the hub API

`HubClient` calls the hub's `/platform/v1` routes with the product's
`platform-<product>` service client. Embed `HubClientConfig` in the product
settings, for example as `platform_client`, and call its `Validate`
implementation. `HubClient::new(&settings.platform_client, &product_id, client)`
takes a `baukit_egress::GuardedClient` built with
`EgressOptions::with_error_body_limit`, so hub error codes reach typed errors.
Construction fails without it.

The client requests a token from `token_url` with `grant_type=client_credentials`,
the client ID and the client secret, and keeps it in memory. Clones share the
cache, and one token request serves concurrent callers. The cached token is
replaced `TOKEN_REFRESH_MARGIN` (30 seconds) before `expires_in` elapses, or at
half its lifetime when that is shorter. A 401 from the API drops that token,
fetches a new one and repeats the request once. A second 401 returns
`HubError::Unauthorized`, as does a token endpoint that refuses the credentials.
The secret lives in `Secret<String>`, so settings debug output redacts it. No
error variant carries a URL, token or secret. `get_json` and `post_json` are
available for other platform routes; use `post_json` only on idempotent routes,
because of the 401 retry.

`HubError::Unavailable` covers transport failures, timeouts, 408, 429 and 5xx.
`InvalidResponse` means a 2xx body that breaks the contract. Other statuses
return `Rejected { status, code }` with the Baukit error envelope code.

### Credits

The `Credits` port has `reserve(subject, amount, idempotency_key)`,
`settle(reservation_id, actual_amount)`, `release(reservation_id)` and
`balance(subject)`. The hub always spends the calling product's wallet. Name the
product's credit actions and keep each action's cost in product configuration,
such as `limits.json`, defaulting to zero. A zero amount returns
`CreditHold::Free` and makes no ledger call. A paid amount returns
`CreditHold::Reserved(reservation)`. Settle or release that reservation when the
action finishes. A repeat with the same key and amount returns the first
reservation, so retries are safe.

`settings.platform.credits(Some(hub))?` returns `HubCredits` in hub mode and
`ZeroCostCredits` in standalone mode, where `hub` may be `None`. `ZeroCostCredits`
accepts every zero-cost action. A configured cost fails with
`CreditsError::WalletUnknown`, because a standalone product has no wallet. Its
balance is zero, and settle and release return `ReservationNotFound`.
`HubCredits` needs the `credits:read` and `credits:spend` scopes. It rejects a
response for another wallet, subject, reservation or amount as
`HubError::InvalidResponse`.

| Hub response | `CreditsError` |
| --- | --- |
| 409 `credits_insufficient` | `Insufficient` |
| 409 `credits_idempotency_conflict` | `IdempotencyConflict` |
| 409 `credits_reservation_expired` | `ReservationExpired` |
| 404 `credits_reservation_not_found` | `ReservationNotFound` |
| 404 `credits_wallet_unknown` | `WalletUnknown` |
| 400 `credits_amount_invalid` | `AmountInvalid` |
| 400 `credits_idempotency_key_invalid` | `IdempotencyKeyInvalid` |
| 404 `platform_subject_not_found` | `SubjectNotFound` |
| 403 `account_inactive` | `AccountInactive` |
| 503 `platform_credits_unavailable`, other 5xx, 408, 429, transport | `Unavailable` |
| Anything else | `Hub(HubError)` |

Amounts above `MAX_CREDIT_AMOUNT` (2^53 - 1) fail locally with `AmountInvalid`.
The balance `revision` counts `credits.changed` events. Plain reserve, settle and
release calls do not emit one. On `PlatformEventKind::CreditsChanged`, call
`balance` before the receipt transaction if the product caches balances.

### Organizations

`HubOrganizations::new(hub)` implements `OrganizationDirectory::summary(id)` with
`GET /platform/v1/organizations/{id}` and the `organizations:read` scope.
`OrganizationSummary` holds `status`, the caller product's `tier`, `features`,
`paid_quantity` (`None` without a seat limit), `assigned_seats` and `revision`.
An unknown or invisible organization returns `OrganizationError::NotFound`. A
summary for another organization or product is `HubError::InvalidResponse`.
`organization.updated` carries only `organizationId` and `revision`, so read the
summary for current values when it arrives.

`HubOrganizations::members(id, cursor)` reads one page of the seat directory,
`GET /platform/v1/organizations/{id}/members?product=<caller>&cursor=<cursor>`.
The product comes from the hub client. Start with `None` and pass each
`next_cursor` until it is `None`:

```rust
let mut cursor = None;
loop {
    let page = organizations.members(id, cursor.as_deref()).await?;
    store_seats(page.items);
    cursor = page.next_cursor;
    if cursor.is_none() {
        break;
    }
}
```

Each `OrganizationMember` has `suite_subject` and `account_role` (`owner`,
`admin`, `billing` or `member`). A malformed or foreign cursor returns
`HubError::Rejected` with code `invalid_pagination`.

For other routes with query parameters, call
`hub.get_json_with_query(&segments, &[(name, value)])`. Names and values are
form-encoded in the given order, path segments are percent-encoded as in
`get_json`, and an empty slice sends no `?`. The call uses the same cached token,
single 401 retry and `HubError` mapping. Runtime Analyzer and
architecture-health-platform can drop their own client-credentials token fetch
and page the seat directory through the shared `HubClient`.

The contract tests in `tests/platform_hub.rs` serve the recorded hub bodies from
[`fixtures/platform-api/v1`](../../fixtures/platform-api/README.md) on a local mock
server. `@baukit/suite-client` exports the matching `CreditBalance`,
`CreditReservation`, `OrganizationSummary` and `PlatformEventPayloads` types.

## Verify tickets

Construct `TicketVerifier` through `settings.platform.ticket_verifier(client)?`
with a `baukit_egress::GuardedClient`. Its address policy controls public, loopback
or explicitly allowed private-network JWKS access. Redirects are rejected and
network deadlines and response size limits come from that client. The verifier
also rejects JWKS bodies above 64 KiB and sets above 64 keys.

`verify(ticket).await` checks Ed25519, key metadata, the exact issuer and product
audience, the claim schema and a seven-day maximum lifetime. `verify_at(ticket,
now)` accepts an explicit Unix-second clock. Expiry is exclusive with no allowance;
`iat` may be at most 60 seconds ahead. Revisions cannot exceed 2^53 - 1, matching
the TypeScript verifier. Verify that the returned `sub` is the subject requesting
access before storing or using the result.

Keys are cached by `kid` for 300 seconds. An unknown key triggers a refresh even
while the cache is fresh. Unknown-key refreshes have a shared five-second cooldown
and concurrent refreshes share a lock. Verification of fresh cached keys does
not wait for a network refresh. Failed fetches also have a five-second
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
| `APP__PLATFORM__SOURCE_APP` | Expected envelope `sourceApp` and `X-Suite-Source`. Required in hub mode. |
| `APP__PLATFORM__EVENT_TYPE_PREFIX` | Optional event prefix when it differs from `source_app`. |
| `APP__PLATFORM__RECEIVER_SECRET` | Current receiver HMAC secret, held as `Secret<String>`. |
| `APP__PLATFORM__PREVIOUS_RECEIVER_SECRET` | Optional previous HMAC secret during rotation. |
| `APP__PLATFORM__STANDALONE_DEFAULT__TIER` | Default standalone tier, initially `free`. |
| `APP__PLATFORM_CLIENT__API_URL` | Hub API base URL, when embedded as `platform_client`. |
| `APP__PLATFORM_CLIENT__TOKEN_URL` | Hub token endpoint, such as `<hub>/realms/suite/protocol/openid-connect/token`. |
| `APP__PLATFORM_CLIENT__CLIENT_ID` | Optional. Defaults to `platform-<product_id>`. |
| `APP__PLATFORM_CLIENT__CLIENT_SECRET` | Service client secret. Server-side only. |

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
    PLATFORM__SOURCE_APP: identity_hub
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
