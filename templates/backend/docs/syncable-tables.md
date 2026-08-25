# Syncable tables

Read this before adding a table that offline clients pull. It is documentation only; nothing in
the generated code depends on it, and a backend with no offline clients can ignore it.

A client pulls incrementally: it remembers the highest revision it has seen and asks for
everything above it. That works only if every syncable row carries a revision from a counter that
is monotonic per owner, and if deletions stay visible.

## Columns

Every syncable table carries these five columns:

| Column | Type | Purpose |
|---|---|---|
| `id` | `UUID PRIMARY KEY` | Client-generated, so a row exists offline before the server sees it. |
| `owner_id` | `UUID NOT NULL REFERENCES users (id)` | The partition a pull is scoped to. |
| `updated_at` | `TIMESTAMPTZ NOT NULL DEFAULT now()` | Last-writer-wins input, if conflicts are resolved that way. |
| `deleted_at` | `TIMESTAMPTZ` | Tombstone. Null means live. |
| `revision` | `BIGINT NOT NULL` | Allocated by `baukit_sync::next_revision` for the write that produced this row state. |

Plus one index per table:

```sql
CREATE INDEX product_records_sync_idx ON product_records (owner_id, revision);
```

It makes an incremental pull a range scan instead of a sort over the owner's whole history.

## Deletions are tombstones

Never `DELETE` a syncable row. Set `deleted_at` and allocate a new revision, exactly like any
other update. A hard delete makes the row stop appearing in pulls, so every client that already
holds it keeps it forever, and the deletion silently never propagates.

## Allocating a revision

`baukit_sync::next_revision` bumps the owner's counter inside the transaction you pass it, so the
allocation commits or rolls back with the row write:

```rust,ignore
let mut transaction = pool.begin().await?;
let revision = baukit_sync::next_revision(&mut transaction, owner_id).await?;
sqlx::query(
    "UPDATE product_records SET name = $1, updated_at = now(), revision = $2 WHERE id = $3",
)
.bind(name)
.bind(revision)
.bind(record_id)
.execute(&mut *transaction)
.await?;
transaction.commit().await?;
```

Allocating outside the write's transaction, or from the pool instead of the transaction, leaves
gaps when the write fails and can hand the same revision to two writers. Call `ensure_owner` once
when a user is created so the counter row exists.

The `sync_revisions` table itself comes from `baukit-sync`'s reference migration; copy
`rust/crates/baukit-sync/migrations/0001_baukit_sync.sql` into this backend's own ordered
migrations and point `owner_id` at your users table.

If an existing counter table still uses `user_id`, copy baukit-sync's
`POSTGRES_RENAME_USER_ID_TO_OWNER_ID_SQL` as a one-shot product migration instead. It preserves
the foreign key and its delete action, renames the conventional constraint, and adds the
`last_revision >= 0` check.

## What stays yours

Baukit provides the counter and this convention, nothing else. Endpoint shapes, payloads,
conflict resolution, batching, and the pull cursor protocol are all product decisions. See
`docs/platform/offline-readiness-contract.md` in the baukit repository for what a product may
claim to the user about sync state.
