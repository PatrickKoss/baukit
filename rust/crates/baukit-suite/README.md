# baukit-suite

`baukit-suite` links two apps for one user, signs events with a per-link HMAC secret,
and delivers them through `baukit-jobs`. Receivers authenticate, validate, deduplicate
and apply an event in one PostgreSQL transaction.

Products embed their own `peers.json` and `catalog.json`. App ids are strings.
The crate has no event builders, reward rules, account tables or UI.

## Features

| Feature | Adds |
| --- | --- |
| `postgres` | Link, inbox and outbox stores; SQL migration constants; erasure adapter |
| `http` | Axum router and `SuiteOpenApi` with the suite routes |
| `jobs` | `SuiteJobHandler` and `run_hourly_cleanup` |

The domain and service ports are available without these features. Transactions
use `&mut sqlx::PgConnection`; services open PostgreSQL transactions through the
link store. Enable all three features for the supplied production adapters.

## Product contracts

Implement `SuiteEventApplier`, `SuiteReplaySource` and `SuiteIdentitySource`.
The applier receives the validated payload, owner, reward mode and replay flag.
It returns `AppliedOutcome` with `Granted`, `NoRule` or `Capped` and an optional
ledger entry id. Replay must return `NoRule` or `Capped` with no ledger entry.

`ValidatedPayload::deserialize<T>()` reads a product-owned type. Put
`#[serde(deny_unknown_fields)]` on that type. Typed builders stay in the product.

Call `SuiteEventOutbox::lock_owner_in_transaction` before locking or writing
product rows. Emit `SuiteEvent`s through `enqueue_in_transaction` before
committing the domain write. Without that owner lock, enqueue returns
`SuiteStoreError::Timeout` on contention and leaves the transaction usable.
Implement identity lookup and history reads on the
provided connection so neither acquires another connection during that transaction.

Use `SuiteConfig` as the product's `suite` configuration section. Call
`validate_for` with the embedded peers and credential cipher at startup. Provide
`Arc<dyn baukit_ratelimit::RateLimitStore>`; use Redis for multiple processes.

Copy `SUITE_MIGRATION_SQL` and `SUITE_RUNTIME_MIGRATION_SQL` after the jobs
migrations for products adding suite tables. Keep applied suite migrations when
their schema matches the shipped SQL, apart from product owner foreign keys.
Migrations do not run on startup. Add any missing owner foreign keys in the product.
Use `PostgresSuiteErasure` and `erase_with_suite` to notify peers before the erasure
transaction starts, then delete suite rows in that transaction. Implement
`SuiteErasureOwnerLookup::lock_owner_in_transaction` with a product owner row
lock. The adapter takes the suite advisory lock first, so domain writes that
use the outbox owner lock finish before cleanup.

See [suite adoption and protocol](../../../docs/platform/suite-events.md) for
configuration keys, public signatures, route mounting, worker wiring, client
integration and the adoption checklist.
