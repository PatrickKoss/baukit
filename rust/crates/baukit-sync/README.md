# baukit-sync

`baukit-sync` allocates per-owner revision numbers for incremental sync, and documents the column
convention a syncable table follows. That is the whole crate.

It is deliberately not a sync engine. Wire payloads, conflict resolution, batching, and the pull
endpoint stay product-owned, as
[the offline readiness contract](../../../docs/platform/offline-readiness-contract.md) says they
should. Baukit owns one mechanism here: handing out the next revision without losing or
duplicating one.

## Why a counter and not a timestamp

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
