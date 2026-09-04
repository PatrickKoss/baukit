# baukit-jobs

`baukit-jobs` provides a durable PostgreSQL job store/outbox and a concurrent
worker runner. Products own job identifiers, JSON payload types, handlers, and
migration order.

Optional owner-scoped enqueue and erasure are not implemented. The accepted
[durable job ownership design](../../../docs/platform/durable-job-ownership.md)
defines the compatibility and migration requirements for that later change.

## Migration

Copy `migrations/0001_baukit_jobs.sql` and
`migrations/0002_baukit_jobs_failure_reason.sql` into the product's own
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
