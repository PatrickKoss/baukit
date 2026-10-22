# baukit-suite

`baukit-suite` links two apps for one user, signs events with a per-link HMAC secret,
and delivers them through `baukit-jobs`. Receivers authenticate, validate, deduplicate
and apply an event in one PostgreSQL transaction.

Products embed their own `peers.json` and `catalog.json`. App ids are strings.
The crate has no event builders, reward rules, account tables or UI.
`PeerRegistry::peer_metadata(&self, id: &str) -> Option<&PeerMetadata>` returns
metadata for the own app or any peer, including inactive peers. Use `scheme`
when building product action links. `metadata()` keeps the full embedded list.

## Features

The default build contains the domain, payload catalog and validation, link protocol,
serde contracts and configuration. It has no PostgreSQL, Tokio or HTTP dependencies.
Use `default-features = false` in domain and ports crates.

| Feature | Adds | Enables |
| --- | --- | --- |
| `postgres` | SQLx connection ports, link/inbox/outbox stores, migration constants and erasure adapter. The stores use the `baukit-jobs` outbox and `baukit-erasure` transaction contract. | None |
| `runtime` | Link, ingest, delivery and erasure services, credential validation, rate limits and Tokio timeouts | `postgres` |
| `delivery` | `ReqwestSuitePeerClient` with guarded HTTP egress | `runtime` |
| `http` | Axum router and `SuiteOpenApi` with the suite routes | `runtime` |
| `jobs` | `SuiteJobHandler` and `run_hourly_cleanup` | `runtime` |

Ports that name `PgConnection` require `postgres`. Other ports and serde types
keep their existing paths without features. `SuiteConfig::validate_for` requires
`runtime` because it accepts a credential cipher. `SuiteConfig::registry` and
ordinary configuration validation work without features.

Enable `http`, `jobs` and `delivery` for the supplied production adapters.
Use `runtime` with a product-owned `SuitePeerClient` to omit the supplied delivery client.

```toml
# Domain and ports crates:
baukit-suite = { version = "0.10.0", default-features = false }
# Product adapter crate:
baukit-suite = { version = "0.10.0", features = ["http", "jobs", "delivery"] }
```

## Hosted authorize pages

Set a full consent URL per authorizing peer. This matches hubs whose route ends
in the authorizer's product ID, and lets each peer move to hosted consent separately:

```toml
[suite.peers.beta]
api_url = "https://beta.example/api/v1"
web_url = "https://beta.example"
authorize_url = "https://account.example/portal/suite/authorize/beta/"
```

The environment key is `<PREFIX>__SUITE__PEERS__BETA__AUTHORIZE_URL`.
`SuiteConfig::validate` rejects invalid URLs during config loading, including
inactive peers. `PeerRegistry::new` repeats the check for programmatic settings.
URLs must be absolute HTTPS, without credentials, query or fragment. HTTP is
allowed only for loopback hosts with `suite.allow_loopback = true`.
The configured path, including a trailing slash, stays intact. `start` appends
`client`, `state`, `code_challenge`, and optional `hint` and `hint_domain`.
Without the override it still returns `<peer web origin>/suite/authorize` with
the same query encoding and ordering.

For programmatic configuration, use
`PeerUrls::new(api_url, web_url).with_authorize_url(full_url)` inside
`PeerRegistrySettings::default().with_public_urls(api_url, web_url).with_peer(id, urls)`.
`with_allow_loopback(true)` enables local HTTP. `SuiteConfig`, `PeerUrls` and
`PeerRegistrySettings` are non-exhaustive; mutate defaults for individual fields.

Products retain their local authorize page and all preview, approve, deny and
callback endpoints. A hosted page calls the authorizer with a product token.
Allow its origin in the product's CORS policy and register its exact token
callback with the product's public OAuth client before switching the URL.
The hosted registry and product registry must agree on product IDs, API bases
and event directions. Callbacks and PKCE exchange still belong to the products.

`@baukit/suite-client` exports `suiteAuthorizeUrl(peer, input, options)` and
`validateSuiteAuthorizePeer(peer, options)`. The peer uses `webUrl` and optional
`authorizeUrl`; input uses `client`, `state`, `codeChallenge`, optional `hint`
and `hintDomain`. Set `options.allowLoopback` only for local development.
Validate peer configuration when loading it. `suiteAuthorizeUrl` repeats the
validation and appends the same protocol parameters as Rust.

## Product contracts

Implement `SuiteEventApplier`, `SuiteReplaySource` and `SuiteIdentitySource`.
The applier receives the validated payload, owner, reward mode and replay flag.
It returns `AppliedOutcome` with `Granted`, `NoRule` or `Capped` and an optional
ledger entry id. Replay must return `NoRule` or `Capped` with no ledger entry.

`ValidatedPayload::deserialize<T>()` reads a product-owned type. Put
`#[serde(deny_unknown_fields)]` on that type. Typed builders stay in the product.
Its `PayloadError` converts to `SuiteStoreError::PayloadInvalid` through `?`.
The inbound HTTP route returns 422 `suite_payload_invalid`; delivery treats
422 as a permanent rejection. Use `domain::validation` field validators to
share bounds with typed parsers. Keep `InvalidData` for corrupt stored data.

Return `Capped` when an owner reached a permanent cap. Ingest records the event
and returns 200, so the sender stops. Return
`SuiteStoreError::QuotaExceeded { retry_after_seconds }` for a quota that
resets. The route returns 429 `suite_quota_exceeded` with `Retry-After` and
rolls back the applier writes. Delivery retries 429 up to ten attempts and
honors `Retry-After` up to 300 seconds. Only a job that exhausts its attempts
counts toward the circuit.

Override `SuiteEventApplier::lock_owner(&mut PgConnection, owner)` when product
writers lock an owner row. Ingest calls this hook after its suite advisory lock
and before locking the link. For example, SLS uses `FOR NO KEY UPDATE` on `users`.
Lock deleted and deactivated owners too and check that state in `apply`.
A filter in `lock_owner` turns connection tests and duplicate redeliveries
for those owners into a permanent 404 `suite_rejected`.
The default hook does nothing, so existing appliers compile unchanged.
See the [crate lock order](https://docs.rs/baukit-suite/latest/baukit_suite/#postgresql-lock-order)
when implementing transaction ports.

Call `SuiteEventOutbox::lock_owner_in_transaction` before locking or writing
product rows. Emit `SuiteEvent`s through `enqueue_in_transaction` before
committing the domain write. Without that owner lock, enqueue returns
`SuiteStoreError::Timeout` on contention and leaves the transaction usable.
Implement identity lookup and history reads on the
provided connection so neither acquires another connection during that transaction.

Use `SuiteConfig` as the product's `suite` configuration section. Call
`validate_for` with the embedded peers and credential cipher at startup. Provide
`Arc<dyn baukit_ratelimit::RateLimitStore>`; use Redis for multiple processes.

Copy `SUITE_MIGRATION_SQL`, `SUITE_RUNTIME_MIGRATION_SQL` and
`SUITE_LOCK_ORDER_MIGRATION_SQL` after the jobs
migrations for products adding suite tables. Keep applied suite migrations when
their schema matches the shipped SQL, apart from product owner foreign keys.
Keep the original filenames and SQLx migration history. Do not copy or run the
crate's `001` and `002` again when their schema is already applied.
Migrations do not run on startup. Add any missing owner foreign keys in the product.
Products upgrading from 0.10.2 add `003_suite_lock_order.sql` as a new product
migration after their applied runtime migration. It removes the link foreign key
from failed-job accounting records and replaces the job failure trigger. Existing
failure records stay accounted; the new store accounts later records before it
reads health or queues work. Deploy this migration before the new store code.

Use `PostgresSuiteErasure` and `erase_with_suite` to check erasure admission before
notifying peers, then delete suite rows in the erasure transaction. Replays and
rejected requests return without revoking the current profile's links.
Preflight makes no writes and takes no locks. The erasure transaction repeats
every check after notification. Two concurrent first requests with the same key
may both notify for the current profile being erased. Implement
`SuiteErasureOwnerLookup::lock_owner_in_transaction` with a product owner row
lock. The adapter takes the suite advisory lock first, so domain writes that
use the outbox owner lock finish before cleanup.

Hourly cleanup logs storage errors and increments `suite_cleanup_failures_total`.
It waits for the next UTC hour boundary after a failure. A database advisory
lock lets one replica run cleanup while the others skip the concurrent run.

## Packaging

The crate archive includes `migrations/001_suite.sql` and
`migrations/002_suite_runtime.sql` and `migrations/003_suite_lock_order.sql`.
These are the only embedded runtime files.
Protocol fixtures ship with `baukit-test` under its `suite` feature; products
provide their own peers and catalog at runtime.

Before the release train publishes the connection-based `baukit-jobs` API,
verify both pending packages together:

```sh
cargo package --manifest-path rust/Cargo.toml -p baukit-jobs -p baukit-suite --all-features
```

Cargo builds the extracted archives against the pending jobs archive. A suite-only
package check against published `baukit-jobs` 0.9.0 uses its older transaction API.

See [suite adoption and protocol](../../../docs/platform/suite-events.md) for
configuration keys, public signatures, route mounting, worker wiring, client
integration and the adoption checklist.
