# baukit-jobs

`baukit-jobs` provides a durable PostgreSQL job store/outbox and a concurrent
worker runner. Products own job identifiers, JSON payload types, handlers, and
migration order.

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
