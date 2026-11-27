# Job deferrals, heartbeats and concurrency keys

`baukit-jobs` 0.32.0 adds two columns and an index to `job_outbox`, widens the `JobStore` trait, and changes the
lease and shutdown defaults. A job can now wait for a dependency without spending an attempt, and a
running attempt renews its lease instead of holding one long lease.

## SQL

Apply the four migrations, in this order, before you deploy the 0.32.0 binary:

| Migration | Constant | Template file | Change |
| --- | --- | --- | --- |
| `0005_baukit_jobs_deferrals.sql` | `POSTGRES_MIGRATION_0005_SQL` | `0012_baukit_jobs_deferrals.sql` | `deferrals INTEGER NOT NULL DEFAULT 0` and its check, `NOT VALID` |
| `0006_baukit_jobs_concurrency_key.sql` | `POSTGRES_MIGRATION_0006_SQL` | `0013_baukit_jobs_concurrency_key.sql` | nullable `concurrency_key TEXT` and its length check, `NOT VALID` |
| `0007_baukit_jobs_validate_checks.sql` | `POSTGRES_MIGRATION_0007_SQL` | `0014_baukit_jobs_validate_checks.sql` | validates both checks |
| `0008_baukit_jobs_running_key_index.sql` | `POSTGRES_MIGRATION_0008_SQL` | `0015_baukit_jobs_running_key_index.sql` | the partial index `job_outbox_running_key_idx`, built `CONCURRENTLY` |

The template file names are the ones the auth and worker templates ship in `backend/migrations/`.
Copy the files after your product's existing baukit-jobs migrations.

All four are expand-only. A 0.31.x worker keeps running against the new columns and ignores them.
None of them blocks writes to `job_outbox` for longer than the 5 second lock timeout. `0008` starts with
`-- no-transaction`, so sqlx runs it outside a transaction. If your migration runner does not read that
marker, run the file on its own, outside a transaction.

The `baukit-suite` delivery worker needs `0006` too, because it sets a concurrency key per peer app.

## `JobStore` implementations

Skip this section if you only use `PostgresJobStore`. A custom `JobStore` must add or change these
methods:

| Method | Signature |
| --- | --- |
| `heartbeat` | `fn heartbeat<'a>(&'a self, lease: &'a JobLease, lease_for: Duration, now: Timestamp) -> StoreFuture<'a, Result<bool, StoreError>>` |
| `release` | `fn release<'a>(&'a self, lease: &'a JobLease, run_after: Timestamp, reason: ReleaseReason, now: Timestamp) -> StoreFuture<'a, Result<Option<FailureDisposition>, StoreError>>` |
| `oldest_pending_age` | `fn oldest_pending_age<'a>(&'a self, job_types: &'a [&'a str], now: Timestamp) -> StoreFuture<'a, Result<Duration, StoreError>>` |
| `queue_depth` | `fn queue_depth<'a>(&'a self, job_types: &'a [&'a str]) -> StoreFuture<'a, Result<QueueDepth, StoreError>>` |

`heartbeat` extends a lease the worker still owns to `now + lease_for` and returns `false` when the job
is no longer running under it. `release` puts an owned running job back to pending at `run_after`.
`ReleaseReason` decides what that costs:

| `ReleaseReason` | Attempt | `deferrals` |
| --- | --- | --- |
| `Shutdown` | given back | unchanged |
| `Deferred` | given back | grows by one |
| `DeferredPastCap` | stays used | unchanged |

`oldest_pending_age` now takes the runner's job types and measures from `MIN(run_after)` of the due
pending rows of those types. Rows scheduled after `now` do not count. `queue_depth` counts pending,
running and failed rows of those types.

## Deferring a job

`JobError::deferred(message, delay)` puts the job back to pending after `delay` and gives the attempt
back. Use it when the job is fine and a dependency is not, for example an open circuit. A job may defer
`WorkerConfig::max_deferrals` times (default 100) for free. Past that, each deferral consumes an
attempt, so a job that defers forever still ends in `attempts_exhausted`.
`worker_job_deferrals_total{job_kind}` counts deferrals.

For outbound calls, [egress-circuit-breaker.md](egress-circuit-breaker.md) shows `egress_job_error`,
which picks the deferral for you.

## Concurrency keys

`NewJob::concurrency_key(key)` tags a job. `WorkerConfig::max_per_key` (`Option<NonZeroU32>`, default
`None`) caps how many running jobs share a key. A claim skips pending rows whose key already holds that
many live leases. Jobs without a key are never limited. Use one key per slow destination, so it cannot
fill every runner slot:

```rust
let job = NewJob::new("invoice.sync", payload, 5).concurrency_key("billing-provider");
```

## Changed defaults

| `WorkerConfig` field | 0.31.x | 0.32.0 |
| --- | --- | --- |
| `lease_duration` | 15 min | 60 s, renewed every third of it |
| `cancellation_poll_interval` | 250 ms | 2 s |
| `idle_poll_max` | new | 5 s |
| `max_deferrals` | new | 100 |
| `max_per_key` | new | `None` |
| `queue_sample_interval` | new | 15 s |
| `release_reserve` | new | 5 s |
| `store_call_timeout` | new | 5 s |

`lease_duration` no longer has to exceed `job_timeout`, because running attempts renew the lease. The
worker template drops `worker.lease_duration_seconds` and its check against `worker.job_timeout_seconds`.
Remove that key and `WORKER__LEASE_DURATION_SECONDS` from your config and GitOps values.

Idle polling starts at `poll_interval` (250 ms) and doubles up to `idle_poll_max`. Enqueue sends
`NOTIFY` on `JOB_ENQUEUED_CHANNEL`. To wake an idle runner at once, run `spawn_enqueue_listener(pool,
wakeup, shutdown)` and pass the same `JobWakeup` to `WorkerRunner::with_wakeup`.

## Store errors and shutdown

The runner no longer stops on a store error. It logs, backs off with full jitter up to 30 s and reports
through `WorkerRunner::store_health()`. Wire that into readiness, as the worker template does:

```rust
let store_health = runner.store_health();
// in the readiness check
if !store_health.is_healthy() {
    return Err(ReadinessError::new("job store calls are failing"));
}
```

Loop claims and queue samples that take longer than `store_call_timeout` fail with the new
`StoreError::Timeout(Duration)`. A lost lease is a warning and `worker_lease_expired_total{queue}`.
`RunnerError::LeaseLost` is gone; remove any match arm on it.

On shutdown the runner cancels running attempts, waits until `release_reserve` before the drain
deadline, then releases the unfinished ones with `ReleaseReason::Shutdown`. Size the worker's drain
with `DrainBudget`, as [http-overload-and-drain.md](http-overload-and-drain.md) describes.

## Recurring jobs

`RecurringJob::new(job_type, interval, max_attempts)` describes a job that runs once per
`FixedUtcInterval` slot. Register it with `WorkerRunner::with_recurring(job)`, which returns
`Result<Self, RunnerError>`. The runner then enqueues the next slot after a slot fails for good, so one
failed run no longer ends the chain. Call `enqueue_initial(store, now)` at startup on every replica. It
skips past a current slot that already failed. The handler still enqueues `next_job(&job, now)` after a
slot succeeds.

## Requeueing failed jobs

`PostgresJobStore::requeue_failed(job_type, ids)` puts failed jobs of one type back to pending, at most
`MAX_REQUEUE_BATCH_SIZE` (1000) ids per call. `baukit-ops` serves it with the `jobs` feature:

```rust
use baukit_ops::jobs_admin::requeue_failed_jobs_router;

let operations = operations.merge(requeue_failed_jobs_router(
    Arc::new(store),
    JOB_TYPES,
    admin_token,
));
```

The route is `POST /admin/jobs/requeue` (`REQUEUE_PATH`) with the body
`{"jobType": "<type>", "ids": ["<uuid>"]}`. It answers `{"requeued": [...]}`. A missing or wrong
`Authorization: Bearer` token gets `401`, a type outside `JOB_TYPES` gets `400`, and a store error
gets `503` with `Retry-After: 5`. Merge it into the operations router only, never the public one.

The worker template mounts it with `mount_requeue_route` behind an in-memory IP limit of 10 calls per
60 s. The route stays off until `worker.admin_token` is set. Copy `mount_requeue_route` from
`templates/backend/backend/crates/__app__-bin/src/lib.rs` and the call from
`templates/worker/backend/crates/__app__-bin/src/bin/worker.rs`.
