# baukit-jobs

`baukit-jobs` provides a durable PostgreSQL job store/outbox and a concurrent
worker runner. Products own job identifiers, JSON payload types, handlers, and
migration order.

Optional owner-scoped enqueue and erasure are not implemented. The accepted
[durable job ownership design](../../../docs/platform/durable-job-ownership.md)
defines the compatibility and migration requirements for that later change.

## Migration

Copy `migrations/0001_baukit_jobs.sql`,
`migrations/0002_baukit_jobs_failure_reason.sql`,
`migrations/0003_baukit_jobs_claim_by_type.sql`, and
`migrations/0004_baukit_jobs_active_deduplication.sql` into the product's own
migrations in that order. Do not point a production service at this crate
directory and do not run schema changes during application startup. A product
may add foreign keys or indexes in a later migration, but the lifecycle columns
and constraints used by the claim query must remain compatible.

Products already using the v0.5.1 schema must add the following as a new
ordered migration (also available as `POSTGRES_MIGRATION_0002_SQL`):

```sql
ALTER TABLE job_outbox
    ADD COLUMN failure_reason TEXT;

UPDATE job_outbox
SET failure_reason = CASE
    WHEN attempts >= max_attempts THEN 'attempts_exhausted'
    ELSE 'permanent'
END
WHERE status = 'failed';

ALTER TABLE job_outbox
    ADD CONSTRAINT job_outbox_failure_reason_value_check CHECK (
        failure_reason IS NULL OR failure_reason IN ('permanent', 'attempts_exhausted')
    ),
    ADD CONSTRAINT job_outbox_failure_reason_status_check CHECK (
        (status = 'failed') = (failure_reason IS NOT NULL)
    );
```

The backfill classifies a legacy failed row as `attempts_exhausted` when
`attempts >= max_attempts`; all other legacy failed rows become `permanent`.

Products that already applied 0001 and 0002 add
`migrations/0003_baukit_jobs_claim_by_type.sql` (also available as
`POSTGRES_MIGRATION_0003_SQL`) as a new ordered migration. It rebuilds
`job_outbox_claim_idx` on `(job_type, run_after, created_at, id)` for pending
rows, so a claim reads only rows of the types it asks for. The migration drops
and recreates the index in one transaction, which blocks reads and writes on
`job_outbox` until it commits.

Products with migrations 0001 through 0003 already applied must copy
`rust/crates/baukit-jobs/migrations/0004_baukit_jobs_active_deduplication.sql`
from the Baukit repository as their next ordered migration. The same SQL is
available as `POSTGRES_MIGRATION_0004_SQL`. Apply it before upgrading the
store, including callers that only use existing enqueue and claim methods.
Returned `Job` values now read both new columns. This is a breaking change for
custom stores or tests that construct `Job`: set `dedupe_key` and
`pending_dedupe_key`, usually to `None`. `Job` remains constructible for custom
`JobStore` implementations. `NewJob` and existing method signatures are unchanged.

The migration adds nullable `dedupe_key` and `pending_dedupe_key` columns. Its
independent partial unique indexes cover pending or running jobs with a
non-null `dedupe_key`, and pending jobs with a non-null `pending_dedupe_key`.
The permanent idempotency index stays unchanged. The CHECK constraints scan
`job_outbox` under `ACCESS EXCLUSIVE`, and the index builds also scan the table
while the migration holds that lock until commit. On a large outbox, split
this into product-owned rollout steps: add the columns and CHECK constraints
with `NOT VALID` in a short transaction, run `VALIDATE CONSTRAINT` separately,
and build each index with `CREATE UNIQUE INDEX CONCURRENTLY` outside a
transaction. Finish all steps before upgrading producers or workers.

Existing rows keep null dedupe keys. The first deduplicated enqueue can create
a duplicate beside an existing pending row after deployment. For idempotent
handlers, a product can accept that extra run. Otherwise pause producers and
workers and backfill the chosen mode's key in a product migration before
switching producers. For example, key only the newest pending `cache.refresh`
row per `resource_id`, even when legacy rows share that key:

```sql
-- Run after 0004, inside the product migration's transaction.
LOCK TABLE job_outbox IN SHARE ROW EXCLUSIVE MODE;
WITH newest AS (
    SELECT DISTINCT ON (payload->>'resource_id')
        id, 'resource:' || (payload->>'resource_id') AS dedupe_key
    FROM job_outbox
    WHERE job_type = 'cache.refresh' AND status = 'pending'
        AND payload->>'resource_id' IS NOT NULL
    ORDER BY payload->>'resource_id', created_at DESC, id DESC
)
UPDATE job_outbox AS job
SET pending_dedupe_key = newest.dedupe_key
FROM newest
WHERE job.id = newest.id;
```

Older pending rows remain unkeyed and will still run unless the product
cancels them. The key expression must match the product's producer exactly.
Use `dedupe_key` for active-job mode and include running rows when backfilling
that mode. Resolve any duplicate keys in its wider set first. Unkeyed legacy
rows remain outside deduplication until the product backfills them or they
finish.
Upgrade workers before enabling pending-only producers so every claim clears
the pending key.

New worker and authenticated backend templates include the same SQL in
`backend/migrations/0007_baukit_jobs_active_deduplication.sql`. Existing products
must add their own migration, without editing one they already applied.

The terminal cleanup and fixed UTC slot APIs added after 0.2.1 need no schema
migration. Cleanup is a `PostgresJobStore` method, so custom `JobStore`
implementations also need no code change.

Use `PostgresJobStore::enqueue_in_transaction` to commit a domain mutation and
its outbox row atomically. If a job's product-side result is stored in the same
PostgreSQL database, use `complete_in_transaction` after writing the result so
both changes commit together. Pass `JobCancellation::worker_id()` as the lease
owner. After the transaction commits, call
`JobCancellation::mark_completed_in_transaction()` and return success
immediately; the runner will record success without attempting a second job
transition. External side effects still need an idempotency key at the
destination because a crashed worker's expired lease is reclaimed.

Handlers classify failures with `JobError::permanent`, `JobError::retryable`,
or `JobError::retryable_after`. The last form preserves a provider's
`Retry-After` signal instead of using the runner's exponential delay; the
job's `max_attempts` remains authoritative. Terminal jobs keep status `failed`
and expose `failure_reason` as `permanent` or `attempts_exhausted` alongside
the bounded diagnostic `last_error`.

## Deduplicate against active jobs

Use `PostgresJobStore::enqueue_deduplicated(job, dedupe_key)` when only one
pending or running job of a type should exist for a key. For a domain mutation
and enqueue that must commit together, call
`enqueue_deduplicated_in_transaction(&mut transaction, job, dedupe_key)`.

```rust,no_run
use baukit_jobs::NewJob;
use serde_json::json;

let outcome = jobs
    .enqueue_deduplicated(
        NewJob::new("cache.refresh", json!({"resource_id": 42}), 3),
        "resource:42",
    )
    .await?;
```

The key is scoped to `job_type`, must contain 1 to 500 characters, and cannot
be blank. Pass a `NewJob` without an idempotency key. Combining an idempotency
key with deduplication returns `StoreError::InvalidInput`. `enqueue` and
`enqueue_in_transaction` still deduplicate by idempotency key across every
status, for as long as that row exists. Permanent, active-job, and pending-only
keys are independent.

| Existing job with the same type and dedupe key | Result |
| --- | --- |
| Pending, including a future schedule or retry | Return it with `created = false` |
| Running, including an expired lease or cancellation request | Return it with `created = false` |
| Succeeded, failed, or cancelled | Insert a new job with `created = true` |
| None | Insert a new job with `created = true` |

A duplicate returns the existing job's payload, schedule, attempt budget, and
lease without changing them. Concurrent enqueues cannot insert two active
rows. The store locks an existing active row until commit and retries if a
conflicting row becomes terminal before the lookup. Keep caller-owned
transactions short. At repeatable read or serializable isolation, callers
must retry the whole transaction after a PostgreSQL serialization failure.

Retries and lease recovery keep the dedupe key. Requesting cancellation of a
running job keeps the key occupied until the job becomes terminal. Cancelling
a pending job releases it immediately. The existing cancellation methods that
take a key match permanent idempotency keys; use the returned job ID to cancel
an active-deduplicated job. `Job::dedupe_key` exposes the key, including on
terminal jobs, for inspection and product-owned queries.

## Deduplicate against pending jobs

Use `PostgresJobStore::enqueue_pending_deduplicated(job, dedupe_key)` when an
update during a running job needs a follow-up. The running job may have read
state before the update. Repeated updates share one pending follow-up without
changing its payload, schedule, or attempt budget. The caller-owned transaction
variant is
`enqueue_pending_deduplicated_in_transaction(&mut transaction, job, dedupe_key)`.
A separate method pair keeps existing signatures and active-job behavior intact.

| Existing pending-only job with the same type and key | Result |
| --- | --- |
| Pending, including a future schedule | Return it with `created = false` |
| Running, with no pending follow-up | Insert one pending job with `created = true` |
| Running, with a pending follow-up | Return the follow-up with `created = false` |
| Succeeded, failed, cancelled, or none | Insert a new job with `created = true` |

`Job::pending_dedupe_key` holds this reservation until the first claim. Claiming
clears it atomically, so another enqueue can create the next follow-up. A claimed
job's retries and lease recovery keep its ID and attempt budget but have no
pending key. A retry can therefore coexist with the keyed pending follow-up;
it does not replace or deduplicate that follow-up. Handlers must tolerate repeat
runs, as they already do for lease recovery. Cancelling a pending job releases
its key immediately. Use the returned job ID for cancellation.

Both method pairs use the same key validation, row locking, concurrency, and
transaction rules. They use separate columns and indexes, so the same type and
key can have one active-job row and one pending-only reservation at once.
Neither mode returns a row created in the other mode. Pick one mode for each
producer flow; changing modes requires a product rollout decision about old jobs.

## Terminal-job cleanup

Call `PostgresJobStore::cleanup_terminal_jobs` with separate `updated_at`
cutoffs for succeeded, cancelled, and failed jobs. The batch size must be from
1 through `MAX_TERMINAL_JOB_CLEANUP_BATCH_SIZE`. One call deletes at most that
many rows in total and returns committed counts for each terminal status.

Cleanup never selects pending or running jobs. This includes a running job with
an expired lease. Rows locked by another transaction are skipped, so one
maintenance pass does not wait for a claim or completion transaction. The
application owns cutoff durations, call frequency, shutdown handling, metrics,
and cleanup for product tables. Repeat bounded calls if a maintenance window
should drain the backlog.

Configure `retain_kinds(&["suite.events.deliver", "suite.links.revoke"])` on
every store used for terminal cleanup when another component owns cleanup for
those kinds. Their succeeded, cancelled, and failed jobs survive every cutoff
and do not count toward the batch limit. Products using `baukit-suite` can pass
`&baukit_suite::SUITE_JOB_TYPES` instead of spelling the kinds.

Use `retain_failed_kinds(&["identity.account.delete"])` to keep only failed jobs
for inspection and repair. Both builders can be set on the same store. Kinds
in both lists are retained in every terminal status.

## Fixed recurring UTC slots

`FixedUtcInterval` calculates whole-second UTC slots anchored at the Unix
epoch. `slot_at` returns the slot containing a timestamp. `next_slot` accepts
the current job's slot and the handler's observed clock time, then returns the
first boundary after both. A job delayed from 12:00 until 12:47 on an hourly
interval schedules 13:00, not 13:47. If several slots were missed, the helper
skips them. If the clock moves backward, it still advances beyond the current
job's slot.

Use `FixedUtcSlot::identifier` as the job's idempotency key and
`FixedUtcSlot::starts_at` as `run_after`. Enqueue the next slot before reporting
the current handler as successful. When the handler writes product data in
PostgreSQL, put those writes, `enqueue_in_transaction`, and
`complete_in_transaction` in one transaction. Commit it, call
`JobCancellation::mark_completed_in_transaction`, and then return success.
An enqueue error must leave the current attempt unsuccessful so the worker can
retry it. Duplicate delivery and process restart calculate the same identifier,
and the existing enqueue contract returns the first row.

The application still owns the interval, initial seed, job type, payload,
catch-up policy, and decision to stop recurring work. `FixedUtcInterval` does
not parse cron expressions or run a scheduler.

## Per-key recurring jitter

Create `FixedUtcJitter::new(fraction)` with a finite fraction in `(0, 1]`, then
call `interval.run_at(slot, key, jitter)` for `run_after`. It adds whole seconds
in `0..=min(floor(interval_seconds * fraction), interval_seconds - 1)`. The run
time stays inside its slot, even with fraction `1.0`. A bound below one second
gives no offset. Timestamp overflow returns
`FixedUtcIntervalError::OutOfRange`.

The offset uses 64-bit FNV-1a over the key's UTF-8 bytes, with offset basis
`14695981039346656037`, prime `1099511628211`, wrapping arithmetic, and modulo
`bound + 1`. The bound uses IEEE 754 binary64 multiplication, capped at one
second less than the interval. The algorithm stays fixed across releases,
platforms, and restarts.
A connection keeps the same offset in every slot. For an hourly interval and
fraction `0.1`, `connection:1` runs 345 seconds after the boundary.

`slot.identifier()` and `next_slot` still use the unshifted UTC boundary.
For per-connection jobs, compose the connection key with `slot.identifier()`
as the idempotency key. Jitter affects only `run_after`.

## Cancellation in a domain transaction

Use `request_cancellation_by_key_in_transaction` for an exact idempotency key,
or `request_cancellation_by_key_prefix_in_transaction` for all keys with a
literal prefix. Both take a caller-owned PostgreSQL transaction, job type,
key or prefix, and timestamp. This lets a connection delete and cancellation
of its poll jobs commit together. Prefixes must contain 1 to 500 characters
and cannot be blank. `%` and `_` match literally.

Pending jobs become cancelled with locks cleared. Running jobs keep their
lease and receive `cancel_requested_at` for the runner's cancellation path.
Terminal jobs and other job types stay untouched. `CancellationOutcome`
reports `cancelled_pending` and `cancellation_requested_running` counts.
Repeat requests count matching running jobs again. Changes become visible
after commit; rollback preserves the jobs.

## Lease checks before intermediate writes

Call `check_lease_in_transaction(transaction, job_id, worker_id, now)` before
writing intermediate product state. It returns true only when the job is
running, the worker owns it, its lease expires strictly after `now`, and no
cancellation was requested. It checks the same predicate as completion and
leaves the lifecycle unchanged.

The check uses `SELECT FOR UPDATE` and holds the row lock until commit or
rollback. Concurrent cancellation and reclaim wait for the lock; claim's
`SKIP LOCKED` selection skips the row. Keep the transaction short. The lock
does not extend the lease deadline, so use a fresh timestamp when completing.

## Claiming by job type

`JobStore::claim` takes the set of job types the caller can handle and claims
only rows whose `job_type` is in that set. `WorkerRunner` passes its handler's
`job_types()` on every claim. Several runners with disjoint handlers can share
one outbox without taking each other's work. An empty set is rejected with
`StoreError::InvalidInput`, as are blank or overlong entries.

A job type that no running handler declares stays `pending` with zero attempts.
`worker_queue_oldest_age_seconds` counts pending rows of every type, so such a
row keeps every runner's queue-age gauge rising and trips the queue-age alert.

Claim recovery still runs for every type. Before selecting a row, a claim
cancels expired leases that carry a cancellation request and fails expired
final attempts, whatever their type. Neither transition runs a handler.
Reclaiming an expired lease for another attempt stays limited to the requested
types.

Custom `JobStore` implementations must apply the same type filter to pending
rows and to expired leases they reclaim.

## Runtime and operations

Create one `WorkerRunner` with a product `JobHandler` and run it in a
`baukit_runtime::TaskSupervisor`. Pass the same process `ShutdownToken` to the
runner. On shutdown it stops claiming and drains in-flight attempts; the
supervisor's shared deadline bounds the drain. Wire `WorkerRunner::ready` (or
`JobStore::ready`) into the worker readiness checks. The probe executes the
`FOR UPDATE SKIP LOCKED` claim-query shape with `LIMIT 0`, so it detects a
missing or incompatible outbox table without consuming work.

The runner emits the telemetry-spec §2.4 families. Job and queue labels come
only from the handler and runner's static identifiers. Configure process
telemetry with `baukit-telemetry`; it owns the real Prometheus histogram buckets
for `worker_job_duration_seconds`.

Store-owned claim, enqueue, and readiness transactions finish in owned tasks even if their
caller is cancelled. Shutdown stops new claims and drains running attempts.
An abandoned claim stays leased until lease expiry, then a matching worker can
recover it. Each such claim consumes an attempt.

Call `retain_failed_kinds(&["identity.account.delete"])` on every store used for
terminal cleanup when identity deletion failures need repair. The exclusion
applies only to failed jobs. Successful and cancelled jobs still expire unless
their kind is also passed to `retain_kinds`.

Transactions passed to the store's transaction methods stay owned by the
caller. These methods do not start or commit a transaction.
