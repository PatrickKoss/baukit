# baukit-sync

`baukit-sync` allocates per-owner revision numbers for incremental sync, purges old tombstones
behind a per-owner horizon, documents the column convention a syncable table follows, and supplies
a hybrid logical clock shared with `@baukit/sync-client`.

It is deliberately not a sync engine. Wire payloads, conflict resolution, batching, and the pull
endpoint stay product-owned, as
[the offline readiness contract](../../../docs/platform/offline-readiness-contract.md) says they
should. The clock orders timestamps. It does not choose which record wins.

## Features

| Feature | Adds | Dependencies |
|---|---|---|
| none | `hlc`, the `horizon` cursor rule and wire constants, and the reference SQL constants | `serde`, `thiserror` |
| `sqlx-postgres` | `next_revision`, `ensure_owner`, `current_revision`, `current_revision_for_update`, and the `purge` module | SQLx 0.9 with PostgreSQL, `chrono`, `uuid` |

A backend that allocates revisions or purges tombstones enables the feature:

```toml
baukit-sync = { version = "0.4", features = ["sqlx-postgres"] }
```

A crate that only needs the clock uses the plain dependency and pulls in no database driver.

## Hybrid logical clock

`hlc::HybridLogicalClock` produces timestamps that remain ordered when its injected physical clock
stalls or moves backward. Callers provide the device ID and physical clock. Callers also load and
save `HybridLogicalClockState`, so the module has no database, device-identity, or random-number
dependency.

```rust
use baukit_sync::hlc::{HybridLogicalClock, HybridLogicalClockState};

let restored: Option<HybridLogicalClockState> = None;
let mut clock = HybridLogicalClock::open("device-a", || 1_700_000_000_123, restored)?;
let local_timestamp = clock.now()?;
let after_remote = clock.observe(local_timestamp)?;
let state_to_persist = clock.snapshot();
# assert!(after_remote > local_timestamp);
# let _ = state_to_persist;
# Ok::<(), baukit_sync::hlc::HlcError>(())
```

The encoding is `wall_time_ms * 1000 + counter + 1`. It matches Redemut's existing Rust and
TypeScript clocks. The added one reserves zero as invalid. `compare` returns `None` if either input
is invalid. A counter that reaches 1,000 moves the wall component forward by one millisecond and
resets the counter to zero.

Encoded values cannot exceed JavaScript's maximum safe integer, `9_007_199_254_740_991`. The last
valid value decodes to wall time `9_007_199_254_740` and counter `990`. A later `now` or `observe`
call returns `HlcError::ExceedsSafeInteger` without changing state.

`open` accepts only state whose device ID matches and whose components encode successfully. It
resets missing, corrupt, or foreign state to `{ wall_time_ms: 0, counter: 0 }`. Persistence read and
write failures remain the caller's errors because persistence stays outside this module.

### Clock migration

Redemut can replace `redemut_services::hlc` with `baukit_sync::hlc`. Its stored integer timestamps
and camel-case serialized state need no data migration. Keep the product's server
compare-and-swap loop, last-writer-wins merge, and device tie-break in Redemut.

## Why revisions use a counter

A client pulls by asking for everything above the revision it last saw. That only works if the
ordering is total and gap-free from the client's point of view, which wall-clock timestamps are
not: two rows written in the same millisecond are indistinguishable, and clock adjustments can
move a row backwards past a cursor the client already passed. A per-owner counter gives a strict
order with no ties.

The counter is per owner rather than global so that one busy account cannot inflate every other
account's cursor, and so two owners' writes never contend on the same row.

## Allocating a revision

Call `next_revision` inside the transaction that writes the row, and stamp the returned value onto
that row:

```rust,no_run
# async fn example(pool: &sqlx::PgPool, owner_id: uuid::Uuid) -> Result<(), sqlx::Error> {
let mut transaction = pool.begin().await?;
let revision = baukit_sync::next_revision(&mut transaction, owner_id).await?;
sqlx::query("UPDATE product_records SET revision = $1, updated_at = now() WHERE id = $2")
    .bind(revision)
    .bind(owner_id)
    .execute(&mut *transaction)
    .await?;
transaction.commit().await?;
# Ok(())
# }
```

`UPDATE ... RETURNING` holds a row lock for the rest of the transaction, so concurrent writers for
one owner serialize and each receives a distinct, increasing value. A rollback discards the
allocation along with the row write, which is the point of taking the caller's transaction rather
than a pool: a revision is never handed out for a write that did not land.

`ensure_owner` creates the counter row and is safe to call more than once. `current_revision`
reads the counter without advancing it, for answering "how far could a pull go".
`current_revision_for_update` takes a row lock for read-dependent writes that must exclude another
revision allocation until the caller's transaction finishes.

## Schema

Copy [`migrations/0001_baukit_sync.sql`](migrations/0001_baukit_sync.sql) into the product's own
ordered migrations, and set `owner_id`'s foreign key to the product's owner table. The crate never
runs migrations at startup.

If an existing `sync_revisions` table uses `user_id`, copy
[`postgres_rename_user_id_to_owner_id.sql`](postgres_rename_user_id_to_owner_id.sql)
instead. It is a one-shot migration for the old shape. It renames the column and conventional
foreign-key constraint, preserves the foreign key and its delete action, and adds the
`last_revision >= 0` check.

Every syncable table carries the same five columns:

| Column | Purpose |
|---|---|
| `id` | Client-generated UUID primary key, so a row can be written offline before the server sees it. |
| `owner_id` | The partition a pull is scoped to. |
| `updated_at` | Last-writer-wins input, if the product resolves conflicts that way. |
| `deleted_at` | Tombstone. A deletion must stay pullable, so rows are marked, never removed. |
| `revision` | The value `next_revision` allocated for the write that produced this row state. |

Each such table also needs `CREATE INDEX <table>_sync_idx ON <table> (owner_id, revision)`, which
turns an incremental pull into a range scan instead of a sort over the owner's whole history.

Deleting a row outright instead of setting `deleted_at` is the bug this convention exists to
prevent: the row simply stops appearing in pulls, and every client that already has it keeps it
forever.

## Tombstone purge horizons

Tombstones cannot stay forever, but deleting one breaks every client whose cursor is still below
it: that client would never learn about the deletion. The fix in
[section 4 of the offline readiness contract](../../../docs/platform/offline-readiness-contract.md)
is a per-owner purge horizon, the greatest revision of any tombstone purged for that owner. A pull
cursor above zero and below the horizon gets `resync_required`, and the client rebuilds from zero.

Copy [`migrations/0002_baukit_sync_purge_horizons.sql`](migrations/0002_baukit_sync_purge_horizons.sql)
after the revision migration. It creates `sync_purge_horizons (owner_id, horizon_revision,
updated_at)`. The table name is fixed, like `sync_revisions`. Its foreign key to `sync_revisions`
cascades, so erasing an owner's counter also removes the horizon.

### Purging

The product supplies one `TombstoneTable` per syncable table. The selector binds the cutoff as `$1`
and the batch limit as `$2`, returns `id`, `owner_id`, and `revision`, and locks its rows with
`FOR UPDATE SKIP LOCKED`. The delete binds the selected ids as `$1`. A child table joins its parent
for `owner_id`, and a parent selector skips rows that still have children:

```rust
use baukit_sync::purge::TombstoneTable;

const LIST_ITEMS: TombstoneTable = TombstoneTable::new(
    "list_items",
    "SELECT item.id, list.owner_id, item.revision
     FROM list_items item JOIN lists list ON list.id = item.list_id
     WHERE item.deleted_at < $1
     ORDER BY item.deleted_at, item.id LIMIT $2
     FOR UPDATE OF item SKIP LOCKED",
    "DELETE FROM list_items WHERE id = ANY($1)",
);

const LISTS: TombstoneTable = TombstoneTable::new(
    "lists",
    "SELECT list.id, list.owner_id, list.revision FROM lists list
     WHERE list.deleted_at < $1
       AND NOT EXISTS (SELECT 1 FROM list_items item WHERE item.list_id = list.id)
     ORDER BY list.deleted_at, list.id LIMIT $2
     FOR UPDATE SKIP LOCKED",
    "DELETE FROM lists WHERE id = ANY($1)",
);

const PURGE_ORDER: [TombstoneTable; 2] = [LIST_ITEMS, LISTS];
```

`purge_tombstones(pool, &PURGE_ORDER, cutoff, limit)` drains the tables in slice order. Each batch
is one transaction that deletes at most `limit` rows and raises each affected owner's horizon to
the greatest revision it removed. The upsert only raises a horizon; a batch of older tombstones
leaves it unchanged. `purge_tombstone_batch` runs one batch inside a transaction the caller owns,
for products that want their own loop. A selector that returns more rows than the limit fails with
`PurgeError::BatchLimitExceeded` before anything is deleted.

A batch never waits for a lock. After selecting candidates it takes `FOR UPDATE SKIP LOCKED` on each
owner's `sync_revisions` row. An owner in the middle of a write, a pull, or another purge is skipped,
and its tombstones stay locked only until the batch commits. The next batch or the next run picks
them up. A drain stops when a batch comes back short or deletes nothing.

Retention periods, table lists, delete order, and scheduling stay in the product. A recurring purge
fits the `baukit-jobs` fixed-slot pattern. Derive the cutoff from the slot, not the wall clock, so
a retried attempt purges exactly what the first attempt would have:

```rust,no_run
use std::{num::NonZeroU32, time::Duration};

use baukit_jobs::{FixedUtcInterval, FixedUtcSlot};
use baukit_sync::purge::{PurgedTable, TombstoneTable, purge_tombstones};
use chrono::{TimeDelta, Utc};

const RECORDS: TombstoneTable = TombstoneTable::new(
    "product_records",
    "SELECT id, owner_id, revision FROM product_records
     WHERE deleted_at < $1 ORDER BY deleted_at, id LIMIT $2
     FOR UPDATE SKIP LOCKED",
    "DELETE FROM product_records WHERE id = ANY($1)",
);
const TOMBSTONE_RETENTION: TimeDelta = TimeDelta::days(30);
const PURGE_BATCH: NonZeroU32 = NonZeroU32::new(1_000).expect("batch size is not zero");
const PURGE_INTERVAL: Duration = Duration::from_secs(60 * 60);

async fn purge_slot(
    pool: &sqlx::PgPool,
    slot: FixedUtcSlot,
) -> Result<(Vec<PurgedTable>, FixedUtcSlot), Box<dyn std::error::Error>> {
    let cutoff = slot.starts_at() - TOMBSTONE_RETENTION;
    let report = purge_tombstones(pool, &[RECORDS], cutoff, PURGE_BATCH).await?;
    let next = FixedUtcInterval::new(PURGE_INTERVAL)?.next_slot(slot, Utc::now())?;
    Ok((report, next))
}
```

Enqueue `next` with `next.identifier()` as the idempotency key and `next.starts_at()` as
`run_after`, as the `baukit-jobs` README describes, before completing the current job.

### Guarding pulls

Call `guard_pull_cursor` first in the pull transaction, then read rows in that same transaction. It
takes `FOR KEY SHARE` on the owner's counter row, which does not block `next_revision`, but keeps a
purge for that owner from committing until the pull ends. Without that lock a purge can commit
between the horizon check and the row reads, and the pull silently drops a deletion.

```rust
use std::collections::BTreeMap;

use axum::http::StatusCode;
use baukit_http::ApiError;
use baukit_sync::horizon::{HORIZON_REVISION_DETAIL, PullCursorError, RESYNC_REQUIRED_CODE};
use baukit_sync::purge::{PullGuardError, guard_pull_cursor};

async fn pull_page(
    pool: &sqlx::PgPool,
    owner_id: uuid::Uuid,
    cursor: i64,
) -> Result<Vec<i64>, ApiError> {
    let mut transaction = pool.begin().await.map_err(ApiError::internal)?;
    guard_pull_cursor(&mut transaction, owner_id, cursor)
        .await
        .map_err(pull_guard_error)?;
    let revisions = sqlx::query_scalar(
        "SELECT revision FROM product_records WHERE owner_id = $1 AND revision > $2
         ORDER BY revision LIMIT 100",
    )
    .bind(owner_id)
    .bind(cursor)
    .fetch_all(&mut *transaction)
    .await
    .map_err(ApiError::internal)?;
    transaction.commit().await.map_err(ApiError::internal)?;
    Ok(revisions)
}

fn pull_guard_error(error: PullGuardError) -> ApiError {
    match error {
        PullGuardError::Cursor(PullCursorError::ResyncRequired { horizon_revision }) => {
            ApiError::new(StatusCode::CONFLICT, RESYNC_REQUIRED_CODE, "Pull again from revision 0")
                .with_details(BTreeMap::from([(
                    HORIZON_REVISION_DETAIL.to_owned(),
                    horizon_revision.into(),
                )]))
        }
        PullGuardError::Cursor(_) => ApiError::validation_field("since_revision", "must not be negative"),
        other => ApiError::internal(other),
    }
}
```

`RESYNC_REQUIRED_STATUS` is 409, the same status as `StatusCode::CONFLICT`.
`check_pull_cursor(cursor, horizon)` is the same rule without a database, for products that load
the horizon another way. Cursor zero is always accepted. A cursor equal to the horizon is valid,
because that client has already seen the purged tombstone's revision.

The response carries only the code and the horizon. Owner IDs, table names, and deleted row data
stay out of it.

### Conformance

`baukit_test::check_purge_horizon_conformance` runs the server side of the contract against a
product adapter: horizon monotonicity, batch bounds, a purge that races an open pull, cursors at
and below the horizon, owner isolation, and erasure. This crate's own tests run it against the
helpers above, and against a pull that reads the horizon outside its transaction to show that the
race case catches it.

## Testing

The integration tests need Docker and are `#[ignore]`d by default:

```bash
cargo test --manifest-path rust/Cargo.toml -p baukit-sync -- --include-ignored
```

They cover monotonic allocation, isolation between owners, rollback returning a revision,
concurrent writers never sharing one, a tombstoned row pulling back in revision order, and the
purge-horizon conformance, table ordering, batch limits, skipped busy owners, and the pull guard.
