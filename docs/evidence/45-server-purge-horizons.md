# Server-side tombstone purge horizons evidence

Plan item 9, "Add server-side tombstone purge horizons". Steps 1 and 2 are implemented. Step 3 is a
read-only comparison below: no product changed. Product adoption is deferred.

## Source revisions

- Baukit baseline `45a1e32`.
- Eigenruhe `f74cebb`. The working tree has uncommitted animation changes only; no file read here
  is touched by them.
- Tiefgang `2d37a06`, clean for the files read.
- Hebkit `841bf5d`, clean for the files read.
- Redemut was read only to see which `baukit-sync` API it imports (`baukit_sync::hlc`).

## Observed duplication

Three products implement section 4 of `docs/platform/offline-readiness-contract.md` on the server,
each in its own way:

- Eigenruhe: table specs in `backend/crates/eigenruhe-postgres/src/retention.rs:10-86`, the batch
  loop and horizon upsert in `:202-280`, `purge_horizon` in `eigenruhe-postgres/src/sync.rs:264`,
  the cursor check in `eigenruhe-services/src/sync.rs:134-137`, the 409 mapping in
  `eigenruhe-api/src/lib.rs:905`, and the scheduler in `eigenruhe-worker/src/retention.rs`.
  Horizon table `sync_purge_horizons (owner_id, revision, updated_at)` from
  `backend/migrations/20260903000001_sync_purge_horizons.sql`.
- Tiefgang: owner discovery, per-owner purge, and `advance_purge_horizon` in
  `backend/crates/tiefgang-postgres/src/retention.rs:354-470`, the horizon read in
  `tiefgang-postgres/src/sync.rs:321-329`, and `guard_pull_cursor` in `sync.rs:2963-2972`. Horizon
  table `sync_purge_horizons (owner_id, horizon_revision DEFAULT 0, purged_at NULL)` from
  `backend/migrations/20260901000015_quotas.sql:9-13`.
- Hebkit: `PURGE_SPECS` and `purge_batch` in
  `backend/crates/hebkit-postgres/src/adapters/postgres/retention.rs:14-116,148-224`, the port
  error `ResyncRequired { horizon_revision }` in `hebkit-ports/src/sync_repository.rs:12`, the
  horizon read inside the pull transaction in `adapters/postgres/sync.rs:4647-4680`, and the
  scheduler in `hebkit-worker/src/retention.rs`. Horizon table
  `sync_tombstone_horizons (owner_id, horizon_revision, updated_at)` from
  `backend/migrations/20260904120000_sync_tombstone_horizons.sql`.

## Baukit owner

`baukit-sync` owns the purge helper, the pull guard, the cursor rule, and the reference migration.
`baukit-test` owns the product-facing conformance check.

## Public types and functions

`baukit-sync`, no feature:

- `horizon::check_pull_cursor(cursor, horizon) -> Result<(), PullCursorError>`.
- `horizon::PullCursorError { Negative, ResyncRequired { horizon_revision } }`, non-exhaustive.
- `horizon::RESYNC_REQUIRED_STATUS` (409), `RESYNC_REQUIRED_CODE` (`resync_required`),
  `HORIZON_REVISION_DETAIL` (`horizon_revision`), and `FULL_RESYNC_CURSOR` (0).
- `POSTGRES_PURGE_HORIZONS_MIGRATION_SQL`, the text of
  `migrations/0002_baukit_sync_purge_horizons.sql`.

`baukit-sync`, feature `sqlx-postgres`:

- `purge::TombstoneTable::new(name, select, delete)`: the caller's selector and delete SQL.
- `purge::purge_tombstone_batch(tx, table, cutoff, limit) -> PurgeBatch { selected, deleted }`.
- `purge::purge_tombstones(pool, tables, cutoff, limit) -> Vec<PurgedTable { table, deleted }>`.
- `purge::purge_horizon(tx, owner) -> Option<i64>`.
- `purge::guard_pull_cursor(tx, owner, cursor) -> Result<(), PullGuardError>`.
- `purge::PurgeError { BatchLimitExceeded { table, selected, limit }, Database }` and
  `purge::PullGuardError { Cursor, Database }`, both non-exhaustive.
- The existing `next_revision`, `ensure_owner`, `current_revision`, and
  `current_revision_for_update` moved behind the feature.

`baukit-test`:

- `PurgeHorizonAdapter` with `create_owner`, `write_live_row`, `write_tombstone`, `purge_batch`,
  `pull`, `purge_horizon`, and `erase_owner`.
- `PurgeHorizonPull { Page { revisions }, ResyncRequired { horizon_revision } }` and `PullPause`.
- `check_purge_horizon_conformance`, `assert_purge_horizon_conformance`, and
  `PurgeHorizonConformanceError`.

## Naming decision

The table is `sync_purge_horizons (owner_id, horizon_revision, updated_at)` with a fixed name, like
`sync_revisions`. It takes Eigenruhe's and Tiefgang's table name, and Hebkit's and Tiefgang's column
name. The column name matches the wire field `details.horizon_revision`, so one name appears in
SQL, Rust, and JSON. Eigenruhe's `revision` would collide in meaning with the row `revision`
column of every syncable table. A caller-supplied table name would force every query to be built at
runtime for no gain: no product needs two horizon tables.

`horizon_revision` has `CHECK (horizon_revision > 0)` and no default. A missing row means no purge,
which is the same as horizon zero, so a zero row carries no information. `updated_at` is not null
and records the last raise. The foreign key points to `sync_revisions (owner_id)` with
`ON DELETE CASCADE`. All three products already cascade `sync_revisions` from their user root, so
erasing the user still removes the horizon, and Baukit does not have to know the user table.

## Contract as implemented

- One batch is one transaction. It selects at most `limit` candidates with the caller's
  `FOR UPDATE SKIP LOCKED`, fails with `BatchLimitExceeded` before deleting anything if the
  selector returned more, then takes `FOR UPDATE SKIP LOCKED` on the candidates' `sync_revisions`
  rows in owner order. It deletes only candidates whose owner it locked and raises those owners'
  horizons with an `UNNEST` upsert whose `ON CONFLICT ... WHERE` only fires for a greater value.
- A purge never waits. Owners in the middle of a write, a pull, or another purge are skipped. That
  rules out lock-order deadlocks between purge workers and request handlers.
- `guard_pull_cursor` rejects a negative cursor without touching the database, locks the owner's
  counter row `FOR KEY SHARE`, then reads the horizon in a second statement. `FOR KEY SHARE` does
  not conflict with the `NO KEY UPDATE` lock that `next_revision` takes, so pulls do not serialize
  with writes. It does conflict with the purge's `FOR UPDATE`, so no purge for that owner can
  commit between the cursor check and the row reads. Reading the horizon in a separate statement
  after the lock avoids a stale join row under `READ COMMITTED` re-evaluation.
- Tables, delete order, retention, dependents, and scheduling stay in the caller. The README shows
  a recurring job that derives the cutoff from a `baukit_jobs::FixedUtcSlot` and schedules the next
  slot with `FixedUtcInterval::next_slot`.

## Cases

Docker-backed, in `rust/crates/baukit-sync/tests/purge_horizons.rs`, driven by the
`baukit-test` check:

- Monotonicity: the horizon equals the greatest purged revision and stays there after a later
  purge of a lower revision.
- Cursor boundaries: cursor zero, a cursor equal to the horizon, and one above it are served; one
  below it gets `resync_required` with the horizon.
- Batch bounds: five tombstones with limit 2 purge as 2, 2, 1, and each batch raises the horizon.
- Concurrent purge and pull: the check pauses a pull after its cursor check and runs a purge for
  that owner. The pull must still return the tombstone, or the purge must wait for it. A variant
  adapter that reads the horizon outside the pull transaction fails this one case and no other.
- Owner isolation: one owner's purge neither moves nor applies another owner's horizon.
- Erasure: deleting the owner's counter row removes its horizon.

Helper-specific tests in the same file cover child-before-parent order, a selector that ignores
its limit, a busy owner that is skipped until its writer commits, an open pull guard that does not
block `next_revision`, the guard's cursor cases, and the table's rejection of a zero horizon and of
an owner with no counter row. Unit tests in `baukit-test` run the check against an in-memory store
with one injected fault per case and assert each fault is reported.

## Step 3: product schemas against the adapter

This is a desk mapping of each product's schema and SQL onto `PurgeHorizonAdapter`. The Docker
variant `Guard::BeforePullTransaction` reproduces the shape that fails.

### Eigenruhe: fail

- Purge: pass. `PURGE_TABLES` selectors already return `owner_id`, `id`, and `revision` by name,
  bind `$1` cutoff and `$2` limit, and lock with `FOR UPDATE SKIP LOCKED`. They map to
  `TombstoneTable` unchanged. The one `delete_dependents` entry becomes a `WITH` clause in that
  table's delete.
- Horizon monotonicity and owner isolation: pass. The `GREATEST` upsert never lowers.
- Owner serialization: fail against section 4. The batch never locks `sync_revisions`, so a purge
  can commit while the owner's pull is open.
- Concurrent purge and pull: fail. `SyncService::pull` reads the horizon through
  `purge_horizon` on the pool (`eigenruhe-services/src/sync.rs:134`), then opens a separate pull
  transaction. A purge that commits between the two drops a deletion for a cursor that passed.
- Erasure: pass. The horizon cascades from `user_identities`.
- Naming: the column is `revision`, the check allows zero, and the horizon has no FK to
  `sync_revisions`.

### Tiefgang: fail

- Purge: pass with a rewrite. Its delete statements are keyed on `user_id` and select through
  `ctid` with `RETURNING revision`. Each maps to a selector `SELECT id, user_id AS owner_id,
  revision ... FOR UPDATE SKIP LOCKED` and a `DELETE ... WHERE id = ANY($1)`. Every synced table has
  a UUID `id`. Retention days become a Rust cutoff instead of `now() - interval`.
- Batch bounds: fail as written. `purge_owner_tombstones` applies the limit to each table for one
  owner, so one transaction can delete eleven times the limit. `baukit-sync` bounds each batch to one
  table and `limit` rows across all owners.
- Owner serialization: pass. It takes a blocking `current_revision_for_update` per owner, which is
  correct but lets a slow writer stall the purge.
- Concurrent purge and pull: fail. `PostgresRepository::pull` reads the horizon on the pool
  (`tiefgang-postgres/src/sync.rs:321-329`) and then reads each table on the pool with no
  transaction at all.
- Erasure: pass through `users` cascade.
- Naming: `horizon_revision` matches. `purged_at` is nullable and the column defaults to zero.

### Hebkit, for reference

Hebkit would pass every case. Its pull reads the horizon inside the pull transaction after
`materialize_catalog_revisions` takes `current_revision_for_update`, which is race-free but makes
every pull wait for every open write for that owner. Its purge locks `sync_revisions` with a
blocking `FOR UPDATE` after it already holds row locks. A request that allocates a revision and
then updates one of those tombstoned rows waits on the purge while the purge waits on it. PostgreSQL resolves the deadlock by aborting one side.
The table is named `sync_tombstone_horizons`.

## Failure behavior

- A database error inside a batch returns `PurgeError::Database`. The caller's transaction rolls
  back, so no delete survives without its horizon. `purge_tombstones` keeps earlier committed
  batches.
- A selector that ignores its limit returns `BatchLimitExceeded` before any delete.
- A drain stops for a table when a batch deletes nothing. If a full batch belongs to busy owners,
  the rest of that table waits for the next run. Retention is a lower bound, so a late purge is
  safe.
- Tombstones whose owner has no `sync_revisions` row are never purged.
- The guard returns `PullGuardError::Cursor` for negative and stale cursors and
  `PullGuardError::Database` otherwise. The product maps the stale case to HTTP 409
  `resync_required` with `details.horizon_revision`, as the README shows with `baukit-http`.

## Privacy boundary

The purge reads ids, owner ids, and revisions and returns counts and table names only. Error
messages carry the table name and counts, never owner ids or row data. The 409 response carries the
code and the horizon, as section 4 requires. `baukit-test` violation messages name the case and
counts only.

## Supported runtimes

Rust 1.95 and later with Tokio. PostgreSQL through SQLx 0.9, only with `sqlx-postgres`. Without the
feature, `baukit-sync` depends on `serde` and `thiserror` only.

## Breaks

- `next_revision`, `ensure_owner`, `current_revision`, and `current_revision_for_update` need
  `features = ["sqlx-postgres"]`. Eigenruhe, Tiefgang, and Hebkit call them and must enable the
  feature.
- `chrono`, `sqlx`, and `uuid` are optional dependencies of `baukit-sync`.
- The purge helpers need migration 0002. A migration that drops `sync_revisions` must drop
  `sync_purge_horizons` first.

## Product code to remove on adoption

- Eigenruhe: `purge_table`, `purge_table_batch`, and the horizon upsert in
  `eigenruhe-postgres/src/retention.rs`; `purge_horizon` in `eigenruhe-postgres/src/sync.rs:264`;
  the cursor check in `eigenruhe-services/src/sync.rs:134-137`.
- Tiefgang: `tombstone_owners`, `purge_owner_tombstones`, `advance_purge_horizon`, and
  `increasing_horizon` in `tiefgang-postgres/src/retention.rs`; `guard_pull_cursor` and the pool
  horizon read in `tiefgang-postgres/src/sync.rs`.
- Hebkit: `purge_batch` and the horizon upsert in `adapters/postgres/retention.rs:148-224`; the
  horizon read in `adapters/postgres/sync.rs:4665-4675`.

## Product adoption follow-ups (deferred)

- Eigenruhe: enable `sqlx-postgres`; migrate `sync_purge_horizons.revision` to `horizon_revision`
  with `CHECK > 0` and an FK to `sync_revisions`; turn `PURGE_TABLES` into `TombstoneTable`s; call
  `guard_pull_cursor` first inside the pull transaction and remove the service-level check; run
  `check_purge_horizon_conformance` in `backend/tests`.
- Tiefgang: enable `sqlx-postgres`; migrate `purged_at` to `updated_at NOT NULL` and drop the zero
  default and zero rows; rewrite the eleven ctid deletes as `TombstoneTable`s in the current
  `PURGE_ORDER`; wrap `pull` in one transaction that starts with `guard_pull_cursor`.
- Hebkit: enable `sqlx-postgres`; rename `sync_tombstone_horizons` to `sync_purge_horizons` and
  repoint its FK; move the `webhook_inbox` dependent delete into a `WITH` clause; replace
  `purge_batch`; swap the horizon read for `guard_pull_cursor`.
- Redemut: no change needed; it imports only `hlc` and stops compiling SQLx once it upgrades.

## Product defects found

- Eigenruhe and Tiefgang: a purge that commits between the horizon check and the row reads makes
  a pull silently skip a deletion. The client keeps a row the server deleted.
- Eigenruhe: the purge does not lock the owner's revision counter, contrary to section 4's "same
  owner serialization".
- Tiefgang: the purge batch limit is per table per owner, not per transaction.
- Hebkit: the blocking `FOR UPDATE` on `sync_revisions` after row locks can deadlock with a
  request that allocates a revision and then updates a selected tombstone.
