# Job claim by type evidence

Plan item 3, "Claim only jobs the worker can handle".

## Source revisions

- Baukit baseline `962ac00`.
- Runtime Analyzer `d47bfd5`, clean working tree.
- Hebkit `841bf5d`, Leitbild `bd38b33`, Tiefgang `2d37a06`, and Eigenruhe `f74cebb` were read
  only to find direct `JobStore::claim` callers, custom stores, and copied claim indexes.

## Observed failure

`PostgresJobStore::claim` selected the oldest due row of any `job_type`. `WorkerRunner` used
`handler.job_types()` only for validation and metric labels. Two worker deployments with
different handlers on one outbox could claim each other's rows. The wrong worker then failed the
row under the `unknown` metric label.

Runtime Analyzer runs two runners in one process
(`backend/crates/finops-bin/src/bin/worker.rs:73-91`), a default queue and an analysis queue,
both with the same `Dispatcher` handler. To keep them apart it forked the whole claim transaction
in `backend/crates/finops-worker/src/queue_store.rs:42-147`. The fork filters with
`job_type = ANY(ANALYSIS_TYPES)` for the analysis queue and `job_type <> ALL(ANALYSIS_TYPES)`
for the default queue (`queue_store.rs:12-17`, `:67`). It filters `oldest_pending_age` the same
way (`queue_store.rs:132-143`), so each queue's gauge reports only its own partition. Its port plan names the gap in
`docs/BAUKIT_PORT_PLAN.md:112`: "no per-type routing". `finops-worker/src/dispatcher.rs:40-44`
and `:202-203` also emit `job_runs_total` and `job_duration_seconds` next to Baukit's
`worker_job_runs_total` and `worker_job_duration_seconds`.

## Baukit owner

`baukit-jobs` owns the claim port, the PostgreSQL claim query, the reference schema, and the
runner. `baukit-test` has no job conformance helper, so it did not change.

## Public types and breaks

- `JobStore::claim(&self, worker_id, job_types: &[&str], now, lease_for)`. The `job_types`
  argument is new and sits after `worker_id`. This breaks every custom `JobStore` and every
  direct caller of `PostgresJobStore::claim`.
- `WorkerRunner::run` passes `handler.job_types()` on every claim.
- `migrations/0003_baukit_jobs_claim_by_type.sql` and `POSTGRES_MIGRATION_0003_SQL` rebuild
  `job_outbox_claim_idx` on `(job_type, run_after, created_at, id) WHERE status = 'pending'`.
- The generated worker template's `backend/migrations/0003_baukit_jobs.sql` creates the new index
  shape directly. Its `docs/durable-jobs.md` gains a section on routing job types to workers.
- A worker whose handler did not declare every type it used to run now leaves the undeclared
  types pending. Before, it claimed them and failed them.

No transition option, fallback, or deprecated signature was added.

## Contract

- A claim selects only rows whose `job_type` is in the requested set. The filter covers pending
  rows and expired leases that would be reclaimed for another attempt.
- Claim recovery stays type-agnostic. Before selecting a row, every claim still cancels expired
  leases that carry a cancellation request and fails expired final attempts. Neither transition
  runs a handler, and the outcome does not depend on which worker performs it. Filtering it would
  leave an exhausted row `running` forever whenever no worker for its type is up.
- `oldest_pending_age` stays unfiltered. A type that no running handler declares stays `pending`
  with zero attempts and keeps every runner's `worker_queue_oldest_age_seconds` rising, which
  trips the existing `BaukitWorkerQueueOldestAgeHigh` alert after ten minutes above 300 seconds.
  The gauge's `queue` label therefore names the reporting runner, not a partition of the outbox.
  Runtime Analyzer's fork filters the age per queue. Baukit does not follow it, because a filtered
  age hides exactly the rows no handler declares, and the plan requires those rows to stay visible.
  A per-type or per-partition age would be a separate gauge, which no surveyed product needs yet.
- `PostgresJobStore::ready` binds an empty type array into the same filtered `LIMIT 0` query, so
  the probe keeps the claim shape without a new parameter.

## Empty set decision

An empty `job_types` set returns `StoreError::InvalidInput`. The alternatives were worse. Treating
empty as "every type" would bring back the cross-claim bug for any caller that forgets the
argument. Returning `Ok(None)` would leave a misconfigured worker idle with no signal.
`WorkerRunner::new` already rejects a handler with no job types, so the runner never reaches this
path. Blank entries and entries over the 200-character `job_type` limit are rejected the same way.

## Index plan

Measured on `postgres:18-alpine`, the image `baukit-test` used at measurement time; its current pin is `18.6-alpine`, with the 0001 and 0002 schema.
The table held 200,000 due pending rows of one unhandled type with older `run_after` values,
1,000 due pending rows each of two handled types, and 50,000 succeeded rows, then `ANALYZE`.
Plans come from `EXPLAIN (ANALYZE, BUFFERS)` on the claim candidate `SELECT`.

| Index                                  | Claim for                 | Plan                                                   | Buffers |
| -------------------------------------- | ------------------------- | ------------------------------------------------------ | ------- |
| old `(run_after, created_at, id)`      | any type (before change)  | Seq Scan, external merge sort of 174,809 rows, 8.2 MB  | 3,607   |
| old `(run_after, created_at, id)`      | one handled type          | Seq Scan, 251,000 rows removed by filter               | 3,607   |
| old `(run_after, created_at, id)`      | two handled types         | Seq Scan, 250,000 rows removed by filter               | 3,601   |
| new `(job_type, run_after, created_at, id)` | one handled type     | BitmapOr of claim and expired-lease indexes, 1,000 rows | 48      |
| new `(job_type, run_after, created_at, id)` | two handled types    | BitmapOr of claim and expired-lease indexes, 2,000 rows | 50      |
| new `(job_type, run_after, created_at, id)` | the backlog's own type | Seq Scan, external merge sort of 172,810 rows, 8.1 MB | 3,601   |

With the old index the filter made no difference: the `OR` with the expired-lease branch already
pushed the planner to a sequential scan, and the type filter only discarded rows after reading
them. With `job_type` leading, a worker reads only its own types' index entries and heap pages,
however large another type's backlog is. The worker that owns the backlog gets the same plan the
unfiltered claim had before, so the change does not regress it. Making that case read the index in
order would need a different query shape, which this item does not attempt.

`postgres_claim_index_skips_a_backlog_of_unhandled_types` pins the result. It seeds 50,000
unhandled rows and 100 handled rows and asserts the plan uses `job_outbox_claim_idx` with no
sequential scan.

The upgrade migration drops and recreates the index in one transaction. That holds an exclusive
lock on `job_outbox` until commit. The reference migration does not use
`CREATE INDEX CONCURRENTLY` because that cannot run inside SQLx's default migration transaction.
A product with a large outbox can apply a concurrent variant with SQLx's `-- no-transaction`
directive.

## Cases

- Isolation: two `WorkerRunner`s with disjoint handlers share one PostgreSQL outbox. Each handler
  sees exactly its own 20 jobs, the `unknown` success counter stays zero, and an aged unhandled
  row stays `pending` with zero attempts.
- Visibility: both runners export `worker_queue_oldest_age_seconds` of at least the unhandled
  row's age.
- Lease expiry: an expired lease of another type is not reclaimed. A matching worker reclaims it
  with the attempt count incremented. An expired final attempt is failed as `attempts_exhausted`
  by a claim for another type, without running a handler.
- Cancellation: a cancelled pending row is not claimable. An expired lease with a cancellation
  request becomes `cancelled`. A runner cancels its filtered running job after a cancellation
  request while an unhandled row stays pending.
- Input: an empty set and a blank entry return `StoreError::InvalidInput`.
- Runner: a unit test with a fake store asserts every claim carries the handler's types and an
  undeclared queued job is never taken.

## Failure behavior

A claim for types with no due rows returns `Ok(None)`, as before. Invalid type sets fail before
the transaction opens. Database errors keep the existing `StoreError::Database` mapping.

## Privacy boundary

Job types are static code identifiers. The change adds no payload data to queries, labels, logs,
or errors.

## Supported runtimes

All targets `baukit-jobs` supports: PostgreSQL through SQLx 0.9, measured on PostgreSQL 18.

## Product adoption

- Runtime Analyzer: delete `backend/crates/finops-worker/src/queue_store.rs` and its
  `pub mod queue_store` line in `finops-worker/src/lib.rs`. Give the two runners in
  `finops-bin/src/bin/worker.rs` a `PostgresJobStore` and two handlers that wrap `Dispatcher`,
  one declaring `ANALYSIS_TYPES` and one declaring the rest of `JOB_TYPES`. The default queue then
  lists its types explicitly instead of taking everything outside the analysis set, so an unknown
  type stays pending instead of being failed. Update `backend/tests/worker_integration.rs`,
  `backend/tests/e2e/pipeline.rs`, and `backend/tests/api_reports.rs`. Delete the
  `job_runs_total` and `job_duration_seconds` registration and recording in
  `finops-worker/src/dispatcher.rs:40-44,202-203` and the `job_runs_total` assertion in
  `backend/tests/worker_integration.rs:401`. Drop the "no per-type routing" gap in
  `docs/BAUKIT_PORT_PLAN.md:112`. The `default` and `analysis` age gauges will both report the
  oldest pending row of any type instead of their own partition.
- Hebkit: pass job types to the direct `claim` calls in `backend/tests/worker_integration.rs`,
  `backend/tests/google_health.rs:110`, and `backend/tests/strava.rs:160`.
- Leitbild: add the `job_types` parameter to `FakeJobStore::claim` in
  `backend/tests/worker_integration.rs:294-303`.
- Eigenruhe, Hebkit, Leitbild, Runtime Analyzer, and Tiefgang: add the 0003 migration after the
  copied baukit-jobs schema, and check that each worker handler's `job_types()` lists every type
  that worker should run.
