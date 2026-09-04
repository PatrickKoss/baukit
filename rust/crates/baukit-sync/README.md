# baukit-sync

`baukit-sync` allocates per-owner revision numbers for incremental sync, documents the column
convention a syncable table follows, and supplies a hybrid logical clock shared with
`@baukit/sync-client`.

It is deliberately not a sync engine. Wire payloads, conflict resolution, batching, and the pull
endpoint stay product-owned, as
[the offline readiness contract](../../../docs/platform/offline-readiness-contract.md) says they
should. The clock orders timestamps. It does not choose which record wins.

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

## Testing

The integration tests need Docker and are `#[ignore]`d by default:

```bash
cargo test --manifest-path rust/Cargo.toml -p baukit-sync -- --include-ignored
```

They cover monotonic allocation, isolation between owners, rollback returning a revision,
concurrent writers never sharing one, and a tombstoned row pulling back in revision order.
