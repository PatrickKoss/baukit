# Add offline sync to a product

At the end of this page your backend stamps every write to a syncable table with a per-owner
revision, serves incremental pulls that never skip a row, purges old tombstones without breaking
clients, and your web or mobile client runs a sync loop against it from a local store partitioned
by account. It is for product developers adding offline support. No template generates sync, so
you assemble it from `baukit-sync`, `@baukit/sync-client` and `@baukit/data-contracts`. The
snippets below come from their READMEs. The Rust ones are doctests, and the crate's integration
tests in `rust/crates/baukit-sync/tests/` run the same calls against PostgreSQL.

Baukit gives you the mechanics. The wire payloads, endpoint paths, conflict rules and the sync
engine stay in the product. The [offline readiness contract](../platform/offline-readiness-contract.md)
says what a product may claim to the user about sync state.

## 1. Add the server pieces

Enable the database feature on the backend crate that writes syncable rows:

```toml
baukit-sync = { version = "0.32", features = ["sqlx-postgres"] }
```

Take the version from the backend's other baukit pins. A crate that needs only the hybrid logical
clock uses the plain dependency and pulls in no database driver.

Copy the reference migrations into the product's own ordered migrations. The crate never runs them:

- `rust/crates/baukit-sync/migrations/0001_baukit_sync.sql` creates `sync_revisions`. Point its
  `owner_id` foreign key at your owner table.
- `rust/crates/baukit-sync/migrations/0002_baukit_sync_purge_horizons.sql` creates
  `sync_purge_horizons`. Add it if you will ever purge tombstones.

Call `baukit_sync::ensure_owner` when you create an owner, so the counter row exists. It is safe
to call more than once.

## 2. Shape each syncable table

A generated backend has these rules in `docs/syncable-tables.md`, rendered from
[`templates/backend/docs/syncable-tables.md`](../../templates/backend/docs/syncable-tables.md).
In short, every syncable table has five columns:

| Column | Type | Purpose |
|---|---|---|
| `id` | `UUID NOT NULL` | Client-generated, so a row exists offline first. |
| `owner_id` | `UUID NOT NULL` | The partition a pull reads. |
| `updated_at` | `TIMESTAMPTZ NOT NULL DEFAULT now()` | Input for last-writer-wins, if you use it. |
| `deleted_at` | `TIMESTAMPTZ` | Tombstone. Null means live. |
| `revision` | `BIGINT NOT NULL` | The value `next_revision` allocated for this row state. |

Key the table `PRIMARY KEY (owner_id, id)` and add `CREATE INDEX <table>_sync_idx ON <table>
(owner_id, revision)`. Clients choose ids, so a bare `id` key lets one owner overwrite another
owner's row. Upserts use `ON CONFLICT (owner_id, id)`. Updates and tombstones put `owner_id` in the
`WHERE`.

Never `DELETE` a syncable row. Set `deleted_at` and allocate a new revision. A hard delete drops
the row from pulls, so clients that hold it keep it forever.

## 3. Stamp each write with a revision

Call `next_revision` inside the transaction that writes the row. From the
[baukit-sync README](../../rust/crates/baukit-sync/README.md#allocating-a-revision):

```rust
let mut transaction = pool.begin().await?;
let revision = baukit_sync::next_revision(&mut transaction, owner_id).await?;
sqlx::query(
    "UPDATE product_records SET revision = $1, updated_at = now() WHERE owner_id = $2 AND id = $3",
)
    .bind(revision)
    .bind(owner_id)
    .bind(id)
    .execute(&mut *transaction)
    .await?;
transaction.commit().await?;
```

The counter row lock serializes writers for one owner, so each gets a distinct, increasing value.
A rollback returns the revision along with the write. Allocating from the pool instead of the
transaction leaves gaps and can hand one revision to two writers.

One push can stamp the same revision on several rows. The pull below handles that.

### Make the push replay-safe

A client that loses the response to a push sends it again. Give push routes an `Idempotency-Key`
and store the first result in the same transaction as the effect. `baukit_http::IdempotencyKeyRule`
parses the header, `baukit_test::check_replay_safe_mutation_conformance` tests the storage path,
and the [replay-safe mutations contract](../platform/replay-safe-mutations.md) has the protocol.
The replay table itself is product code.

### Cap live rows

If a table has a per-owner or per-parent cap, a `COUNT(*)` before the insert races under READ
COMMITTED. Follow the [live-row caps recipe](../platform/live-row-caps.md) and test it with
`baukit_test::check_postgres_live_row_cap_conformance`. The
[resource budget contract](../platform/resource-budgets-contract.md) lists the other limits every
write path declares.

## 4. Serve the pull

A pull runs in one transaction:

1. `baukit_sync::pull::open_pull_window(&mut transaction, owner_id, after)` checks the cursor
   against the purge horizon and returns a window with `hi`, the owner's counter at the start.
   It takes `FOR KEY SHARE` on the counter row. Writers are not blocked, but a purge for that
   owner waits until the pull commits.
2. Each table query filters `owner_id = $1 AND revision <= hi AND (revision, id) > cursor`,
   ordered by `(revision, id)`, with its own `LIMIT`.
3. `merge_pull_page` combines the per-table pages. It ends the page at the lowest last row among
   tables that filled their limit, drops rows above it and sets `more`. When no table fills its
   limit, the cursor is `window.drained_cursor()`.

The `(revision, id)` cursor is what makes shared revisions safe. With a revision-only cursor, a
page that ends inside a revision skips the rest of it. The `revision <= hi` filter stops a writer
that commits between two table reads from pushing the cursor past a row the client never saw.

The README doctest under
[Guarding pulls](../../rust/crates/baukit-sync/README.md#guarding-pulls) is a complete
`pull_page` function for two tables, including the error mapping. Copy it rather than retyping it.
Its error mapping turns `PullCursorError::ResyncRequired` into this response:

- status 409, `RESYNC_REQUIRED_STATUS`
- code `resync_required`, `RESYNC_REQUIRED_CODE`
- detail `horizonRevision`, `HORIZON_REVISION_DETAIL`, holding the horizon

Owner ids, table names and deleted data stay out of the response. A client that stored a
revision-only cursor converts it with `PullCursor::after_revision(n)`. `PullCursor::START` is the
full rebuild cursor.

`guard_pull_cursor` still exists but returns no window. New code uses `open_pull_window`.

Test the pull against your own SQL with `baukit_test::check_pull_conformance`. It seeds several
rows per revision and checks that paging returns each row once.

## 5. Purge old tombstones

Skip this step if you keep tombstones forever. Otherwise each purge raises a per-owner horizon,
the greatest purged revision. A cursor above zero and below the horizon gets `resync_required`,
and the client rebuilds from zero.

Describe each syncable table as a `baukit_sync::purge::TombstoneTable`. The selector binds the
cutoff as `$1` and the batch limit as `$2`, returns `id`, `owner_id` and `revision`, and locks with
`FOR UPDATE SKIP LOCKED`. The delete binds the selected ids as `$1` and their owners as `$2`. List
children before parents. The
[purging section](../../rust/crates/baukit-sync/README.md#purging) of the README has a parent and
child example as a doctest.

Run `purge_tombstones(pool, &tables, cutoff, limit)` from a recurring job. Each batch is one
transaction that deletes at most `limit` rows and raises each affected owner's horizon. An owner
that is mid-write, mid-pull or mid-purge is skipped and picked up next time. Derive the cutoff from
the job's slot, not the wall clock, so a retried attempt purges the same rows. The README's
`purge_slot` doctest does this with `baukit_jobs::FixedUtcSlot`. The [jobs guide](jobs.md) shows
how to enqueue the next slot.

`baukit_test::check_purge_horizon_conformance` checks horizon growth, batch bounds, a purge racing
an open pull, owner isolation and erasure against your adapter.

### Erasing an owner who can come back

Deleting the `sync_revisions` row restarts the counter at zero. `open_pull_window` answers a cursor
above the counter with `resync_required`, but only until new writes pass the old cursor. If an
owner id can return, keep the counter. In one transaction delete the rows, allocate one more
revision and raise the owner's `sync_purge_horizons` row to it. The [privacy guide](privacy.md)
covers erasure in general.

## 6. Partition the local store by account

On the device, user data lives in a store whose name derives from the account. The
[local-data ownership contract](../platform/local-data-ownership-contract.md) makes this a security
boundary. The wrong account must not read another account's local data before sync even starts.

`@baukit/data-contracts` provides the pieces:

- `deriveScopedStoreName(namespace, subject)` hashes the identity with SHA-256 into an opaque store
  name. Pass the server profile id confirmed by `/me` as `subject`, not the OIDC `sub`.
- `resolveScopedStore` keeps the profile-to-store mapping in a `ScopedPersistenceRegistryStore`
  outside the domain database, and throws `PersistenceIdentityMismatchError` on any malformed or
  inconsistent entry.
- `ScopedPersistenceLifecycle` closes the old store before it opens another, resets user-scoped
  memory, and publishes only a hydrated partition. `handleSessionExpired` and
  `handleProfileErased` handle the terminal cases.
- `recheckServerSubjectBeforeSyncAdoption` runs right before an outbox push.

Storage adapters implement `TransactionalStorageStore`. Baukit ships
`@baukit/data-contracts-dexie` for IndexedDB and `@baukit/data-contracts-expo-sqlite` for Expo.
A custom adapter proves itself with the suites from `@baukit/data-contracts/vitest`, such as
`describeTransactionalStorageContract` and `describeScopedPersistenceContract`.

For a single server document edited in place, rather than a synced collection, use
`createRevisionedWriteQueue` from `@baukit/data-contracts/revisioned-writes`. It sends one write at
a time against an expected revision and keeps unknown outcomes apart from newer edits.

## 7. Run the client sync loop

`@baukit/sync-client` has no runtime dependencies at its root. You write the engine. The package
supplies these parts around it.

### Schedule runs

`SyncScheduler` takes one `run()` callback and an environment. It allows one active run at a time.
It runs on start, on return to the foreground, when connectivity returns, and on an interval. From
the [sync-client README](../../typescript/packages/sync-client/README.md#expo-environment):

```ts
import { SyncScheduler } from '@baukit/sync-client';
import { createExpoSyncEnvironment } from '@baukit/sync-client/expo';

const scheduler = new SyncScheduler(() => engine.run(), createExpoSyncEnvironment());
```

A web app uses `createBrowserSyncEnvironment` from `@baukit/sync-client/browser`. A local write
calls `scheduler.requestFollowUp()`, which queues one more run if a run is active. Pass a `retry`
option to back off after a failure, with `fullJitterBackoffMs` from `@baukit/api-runtime/backoff`
as the delay policy. The README has the full retry example.

### Talk to the server

`SyncTransport` wraps an injected `fetch` or an existing API client's request function. It turns
failures into typed errors: `SyncAuthError` for 401, `SyncRateLimitError` for 429 with `retryAt`,
`SyncNetworkError` for transport failures and timeouts, `SyncServerError` for other statuses, and
`SyncPartitionMismatchError` when the body carries the partition-mismatch code. Every request has
a 30 second deadline by default, `DEFAULT_REQUEST_TIMEOUT_MS`. Pass `partitionId` to send the
partition header with a push.

### Push in dependency order

`rankPushBatch` groups queued changes per entity, keeps the newest change for each, and orders the
batch so a parent never follows its children. `isHeldBack` drops a parent whose required children
are still unsent. `coveredChangeIds` lists every outbox row one server outcome settles.

Before you acknowledge anything, call `validatePushOutcomeCoverage`. It throws if a submitted
entity has no outcome.

### Apply a pull page

Call `validatePullPage(currentCursor, page, compare)` before following `hasMore`. It throws on a
cursor that regresses or does not move. Then apply the rows with
`commitCursorAfterLocalTransaction`, which stores the cursor only after the local transaction
resolves. A failed local apply then cannot skip remote changes.

On a 409 `resync_required`, clear server-backed rows and write the zero cursor in one local
transaction, keep pending writes and rejection records, and pull from zero.

### Show status

`SyncStatusStore` is the observable sync status. Call `setSyncing`, `setIdle`, `setFailure` and
`setAttention` from the engine. It applies the contract's ranking: an actionable rejection
outranks a transport failure, and unsent work outranks a bare error. `deriveInitialSyncState`
maps a snapshot to `unknown`, `syncing`, `offline-cached`, `sync-error-cached` or `settled`, so a
cold start does not flash an empty screen. Persist both timestamps and restore them with
`status.hydrate`.

### Order timestamps

If your conflict rule needs ordered timestamps, `HybridLogicalClock` from
`@baukit/sync-client/hlc` and `baukit_sync::hlc` share one integer encoding,
`wall_time_ms * 1000 + counter + 1`. Rust and TypeScript compare stored values directly. The clock
orders timestamps. Choosing the winning record is still your code.

## 8. Prove the client

`createSyncConformanceTests` from `@baukit/sync-client/conformance` returns plain async cases for
any test runner. Your adapter supplies two isolated clients, a fake server, and the wire and
local-storage callbacks. The cases cover replay, rollback, cursor paging, stale pulls over pending
writes, dependency order and two-client convergence with a tombstone. Set the optional
`fullResync` field once you purge tombstones, and extra cases cover the horizon and the reset.
The [conformance section](../../typescript/packages/sync-client/README.md#conformance-harness)
lists every callback.

## Reference

- [baukit-sync README](../../rust/crates/baukit-sync/README.md)
- [baukit-sync integration tests](../../rust/crates/baukit-sync/tests/)
- [baukit-test README](../../rust/crates/baukit-test/README.md)
- [`@baukit/sync-client` README](../../typescript/packages/sync-client/README.md)
- [`@baukit/data-contracts` README](../../typescript/packages/data-contracts/README.md)
- [`@baukit/data-contracts-dexie` README](../../typescript/packages/data-contracts-dexie/README.md)
- [`@baukit/data-contracts-expo-sqlite` README](../../typescript/packages/data-contracts-expo-sqlite/README.md)
- [Template syncable tables notes](../../templates/backend/docs/syncable-tables.md)
- [Offline readiness contract](../platform/offline-readiness-contract.md)
- [Local-data ownership contract](../platform/local-data-ownership-contract.md)
- [Replay-safe mutations](../platform/replay-safe-mutations.md)
- [Live-row caps](../platform/live-row-caps.md)
- [Resource budget contract](../platform/resource-budgets-contract.md)
- [`baukit-offline-sync` skill](../../agent-skills/skills/baukit-offline-sync/SKILL.md)
- [ADR 0022: client-local data partitions](../adr/0022-client-local-data-partitions.md)
- [ADR 0023: sync revisions and pull window](../adr/0023-sync-revisions-and-pull-window.md)
