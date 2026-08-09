# baukit-jobs

`baukit-jobs` provides a durable PostgreSQL job store/outbox and a concurrent
worker runner. Products own job identifiers, JSON payload types, handlers, and
migration order.

## Migration

Copy `migrations/0001_baukit_jobs.sql` into the product's own migrations. Do
not point a production service at this crate directory and do not run schema
changes during application startup. A product may add foreign keys or indexes
in a later migration, but the lifecycle columns and constraints used by the
claim query must remain compatible.

Use `PostgresJobStore::enqueue_in_transaction` to commit a domain mutation and
its outbox row atomically. If a job's product-side result is stored in the same
PostgreSQL database, use `complete_in_transaction` after writing the result so
both changes commit together. External side effects still need an idempotency
key at the destination because a crashed worker's expired lease is reclaimed.

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

