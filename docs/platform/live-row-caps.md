# PostgreSQL live-row caps

**Status:** Product-owned SQL recipe with shared conformance checks.
**Applies to:** PostgreSQL tables whose live rows have a finite cap per owner, parent, or time bucket.
**Related:** [resource budget contract](./resource-budgets-contract.md).

## Define the transition before choosing SQL

A live row has `deleted_at IS NULL`. Creating a live row and restoring a tombstone consume one slot.
Updating a row within the same scope consumes no slot. Soft deletion releases one slot. Moving a live
row to another owner, parent, or day is a release in the old scope and a create in the new scope. Make
both changes in one transaction.

Products own the table, columns, scope, cap, day boundary, and stable rejection code. A per-day cap
should store or derive one documented date, such as a UTC date. Do not let a session timezone setting
change the bucket after rows have been written.

The common unsafe form is a `COUNT(*)` followed by an insert under PostgreSQL's default `READ
COMMITTED` isolation. Two transactions can read the same count and both insert. Putting the queries
in one transaction does not close that race.

## Choose one enforcement method

| Method | Tombstones and supporting index | Update at capacity | Rejection mapping |
| --- | --- | --- | --- |
| Lock a stable scope row | Count only `deleted_at IS NULL`. Use a partial index beginning with every scope column, such as `(owner_id)`, `(owner_id, parent_id)`, or `(owner_id, cap_day)`. | Skip the count when the target is already live in the same scope. A move or restore takes both scope locks in a fixed order. | After the lock, `count >= cap` maps directly to the product's stable limit code. Database failures remain availability failures. |
| Serializable transaction | Count only live rows with the same partial indexes as row locking. Tombstones remain outside the count. | An in-place update writes no new predicate member and succeeds. A move or restore is a capacity transition. | Retry SQLSTATE `40001` with a finite attempt limit. A retry that observes the cap maps to the stable limit code. Exhausted serialization retries do not pretend to be a limit rejection. |
| Maintained counter | Store one `live_count` per exact scope. The counter row's primary key supports reservation. Keep a partial row index for audits and ordinary reads. Soft deletion decrements once. | Do not change the counter for an in-place update. Scope moves reserve the new scope and release the old one in the row transaction. | An `UPDATE ... WHERE live_count < $cap RETURNING` miss maps to the stable limit code. Insert or counter failures roll back together. |
| Database slot constraints | Give each live row a required numbered slot. A fixed-range `CHECK` and partial unique index on `(scope_columns, cap_slot) WHERE deleted_at IS NULL` bound live rows. Tombstones free their slot. | Keep the same slot for an in-place update. A restore or scope move allocates a slot again. | Retry SQLSTATE `23505` only when PostgreSQL names the live-slot index. A retry that finds no free candidate maps to the stable limit code. Other uniqueness failures retain their normal meaning. |

Row locking is the clearest default when a stable owner or parent row already exists. Lock that row,
not the matching child rows. PostgreSQL cannot lock a row that does not exist, so `SELECT ... FROM
children WHERE ... FOR UPDATE` does not serialize two creates into an empty scope. A per-day lock can
use a product-owned bucket row, or a coarser owner lock if that contention is acceptable.

Serializable transactions avoid a counter or lock table, but every caller needs bounded retry logic.
Maintained counters make the boundary check cheap and work with configurable caps, at the cost of a
derived value that needs an audit query and a repair procedure. Slot constraints are useful for small,
fixed caps. Changing the cap usually needs a migration, and every write path must preserve slot
assignment.

## SQL shapes

The examples use literal identifiers on purpose. Each product writes SQL for its own schema.

Lock a stable scope row, count through the partial index, and insert before commit:

```sql
BEGIN;
SELECT id FROM owners WHERE id = $1 FOR UPDATE;
SELECT count(*)
FROM documents
WHERE owner_id = $1 AND deleted_at IS NULL;
-- Return the product limit error when count >= $2.
INSERT INTO documents (id, owner_id, deleted_at) VALUES ($3, $1, NULL);
COMMIT;

CREATE INDEX documents_live_owner_idx
    ON documents (owner_id)
    WHERE deleted_at IS NULL;
```

For a parent cap, use `(owner_id, parent_id)` in the count index and lock the parent row. Include the
owner in both the lock predicate and foreign key so one owner's request cannot lock or write another
owner's parent. For a day cap, use `(owner_id, cap_day)` and lock a stable scope row that includes the
same day if owner-wide serialization is too coarse.

Serializable enforcement uses the same count and index:

```sql
BEGIN ISOLATION LEVEL SERIALIZABLE;
SELECT count(*)
FROM documents
WHERE owner_id = $1 AND deleted_at IS NULL;
-- Return the product limit error when count >= $2.
INSERT INTO documents (id, owner_id, deleted_at) VALUES ($3, $1, NULL);
COMMIT;
```

Retry the complete transaction on `40001`. Do not retry only the insert, and do not map `40001`
itself to the public limit code.

A counter reserves capacity and inserts the row in one transaction:

```sql
BEGIN;
UPDATE document_capacity
SET live_count = live_count + 1
WHERE owner_id = $1 AND live_count < $2
RETURNING live_count;
-- No returned row means the cap is full.
INSERT INTO documents (id, owner_id, deleted_at) VALUES ($3, $1, NULL);
COMMIT;
```

Soft deletion must decrement only when it changed a live row:

```sql
BEGIN;
WITH deleted AS (
    UPDATE documents
    SET deleted_at = now()
    WHERE id = $1 AND owner_id = $2 AND deleted_at IS NULL
    RETURNING owner_id
)
UPDATE document_capacity AS capacity
SET live_count = capacity.live_count - 1
FROM deleted
WHERE capacity.owner_id = deleted.owner_id;
COMMIT;
```

Add `CHECK (live_count >= 0)`. Periodically compare the counter with `COUNT(*) FILTER (WHERE
deleted_at IS NULL)`. A repair must lock the scope and block capacity-changing writes while it resets
the value.

For a fixed cap of ten, a slot constraint can enforce the bound without a counter:

```sql
ALTER TABLE documents
    ADD COLUMN cap_slot smallint,
    ADD CONSTRAINT documents_cap_slot_range
        CHECK (cap_slot BETWEEN 1 AND 10),
    ADD CONSTRAINT documents_live_cap_slot_required
        CHECK (deleted_at IS NOT NULL OR cap_slot IS NOT NULL);

CREATE UNIQUE INDEX documents_live_owner_slot_idx
    ON documents (owner_id, cap_slot)
    WHERE deleted_at IS NULL;
```

Backfill slots before validating these checks on an existing table. Select a free slot and insert in
one transaction. Concurrent allocators may choose the same slot. Retry only `23505` for
`documents_live_owner_slot_idx`, then select again. A retry that finds no free slot returns the stable
limit code. If named conflicts exhaust a finite retry allowance while another slot may still be free,
return a retryable or availability error instead. A partial unique index is reported through
PostgreSQL's constraint-name field using its index name.

## Run the shared race check

Implement `PostgresLiveRowCapAdapter` around a clean scope and a connection pool. The two
`create_row` calls in the final-slot race are polled concurrently, so they must be able to acquire
separate connections.

```rust,no_run
# async fn check<Adapter, ProductError>(
#     adapter: &Adapter,
# ) -> Result<(), baukit_test::LiveRowCapConformanceError>
# where
#     Adapter: baukit_test::PostgresLiveRowCapAdapter<Error = ProductError>,
#     ProductError: ProductLimitCode,
# {
use baukit_test::{
    LiveRowCapConformanceCases, check_postgres_live_row_cap_conformance,
};

check_postgres_live_row_cap_conformance(
    adapter,
    LiveRowCapConformanceCases::new(100, "row_cap_per_owner"),
    ProductLimitCode::limit_code,
)
.await
# }
# trait ProductLimitCode {
#     fn limit_code(&self) -> Option<&str>;
# }
```

The helper requires an empty scope, fills `limit - 1` rows, races two creates, and requires one
accepted create and one stable-code rejection. It checks the live count after the race, updates a row
at capacity, soft-deletes that row, creates a replacement, and checks the count again. Its errors omit
adapter error text, row values, and scope identifiers.

The older `LiveRowLimitAdapter`, `check_update_at_capacity`, and
`check_soft_delete_capacity_reuse` APIs remain available. They test sequential behavior. Add the
PostgreSQL race check when migrating an existing test suite.

## Measured PostgreSQL behavior

`baukit-test` runs the four methods against PostgreSQL 18 Alpine in Docker. The ignored test
`live_row_cap::tests::compares_live_row_cap_methods_on_postgres` synchronizes two independent
operations at the last slot and repeats each race 16 times.

The unsafe `READ COMMITTED` count-then-insert control accepted both creates in all 16 runs, leaving
three live rows under a cap of two. Each enforcement method accepted exactly one create in all 16
runs. Row locking produced 16 post-lock capacity reads. Serializable enforcement observed 16
`40001` failures and turned each bounded retry into a capacity rejection. Counter enforcement
produced 16 conditional-update misses. Slot constraints produced 16 `23505` conflicts naming the
partial unique index. For every method, the same test updated at capacity, soft-deleted a row, and
created its replacement.

These results prove the forced boundary interleaving on the pinned test image. They are not throughput
benchmarks. Measure contention with the product's row sizes, scope distribution, and transaction
work before choosing between a coarse row lock and the other methods.

## Migration

Replace any unprotected count-then-insert sequence before relying on the cap for storage control.
Choose one method per scope, add its index or counter schema, and apply it to every create, restore,
scope move, soft delete, import, and sync write. Preserve the current stable reason code at REST,
sync, import, and other ingress boundaries. A database error becomes that code only when the method
has identified capacity exhaustion.

Keep dynamic table and column names in product code. This recipe and `baukit-test` check behavior;
they do not add schema or SQL ownership to `baukit-sync`.
