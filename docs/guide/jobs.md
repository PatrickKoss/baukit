# Run durable jobs in a worker

At the end of this page your product writes a job in the same transaction as its domain change, a
separate worker process claims and runs it, recurring work schedules itself, old jobs get cleaned
up, and the worker runs in the cluster with alerts. It is for product developers who generate a
backend with `--worker` or add `baukit-jobs` by hand.

## Generate the worker

`--worker` needs `--backend`:

```bash
baukit new my-product --backend --worker
```

The worker capability adds these files to the backend workspace:

- `backend/crates/<app>-worker/src/lib.rs`, the job handler. From `templates/worker/backend/crates/__app__-worker/src/lib.rs`.
- `backend/crates/<app>-bin/src/bin/worker.rs`, the worker process. From `templates/worker/backend/crates/__app__-bin/src/bin/worker.rs`.
- `backend/migrations/0003_baukit_jobs.sql` and `0007_baukit_jobs_active_deduplication.sql`, the product-owned copy of the outbox schema.
- `backend/tests/worker_integration.rs`, a PostgreSQL integration test.
- `docs/durable-jobs.md`, the product-side maintenance notes.

`make run-worker` starts the worker locally. `baukit.toml` records the capability as `worker = true`.

## Own the schema

The `job_outbox` table belongs to the product. `baukit-jobs` never runs migrations at startup.
A product that adds the crate without the template copies the reference migrations from
`rust/crates/baukit-jobs/migrations/` into its own ordered migrations, `0001` through `0004` in
order. The crate README lists the extra steps for products that applied an older subset, and how
to split `0004` on a large outbox.

## Enqueue in the domain transaction

`PostgresJobStore::enqueue_in_transaction` writes the outbox row in a transaction the caller owns.
The domain row and the job then commit or roll back together. The generated item repository does
this when it creates an item, in `templates/backend/backend/crates/__app__-postgres/src/lib.rs`:

```rust
PostgresJobStore::new(self.pool.clone())
    .enqueue_in_transaction(
        &mut transaction,
        NewJob::new(ITEM_CREATED_JOB_TYPE, payload, 3)
            .idempotency_key(format!("item-created:{}", created.id)),
    )
    .await
    .map_err(RepositoryError::unavailable)?;
```

`NewJob::new` takes the job type, a JSON payload and the attempt budget. The idempotency key
deduplicates across every status for as long as the row exists, so a retried request does not
enqueue twice.

Two other modes deduplicate against a narrower set:

- `enqueue_deduplicated` returns the existing pending or running job with the same type and key.
- `enqueue_pending_deduplicated` allows one pending follow-up while a job with that key runs. Use it
  when an update during a run needs another pass.

Both have `_in_transaction` variants. Neither accepts a `NewJob` that also carries an idempotency
key. The tables of outcomes are in the
[baukit-jobs README](../../rust/crates/baukit-jobs/README.md#deduplicate-against-active-jobs).

## Write the handler

A handler implements `JobHandler`. `job_types()` returns static strings. The runner claims only
those types, and they become the `job_kind` metric label, so never build them from payload data.
The generated handler, from `templates/worker/backend/crates/__app__-worker/src/lib.rs`:

```rust
impl JobHandler for DemoJobHandler {
    fn job_types(&self) -> &'static [&'static str] {
        JOB_TYPES
    }

    fn handle<'a>(
        &'a self,
        job: &'a ClaimedJob,
        cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(async move {
            if job.job_type != ITEM_CREATED_JOB_TYPE {
                return Err(JobError::permanent("unsupported demo job type"));
            }
            if cancellation.is_cancelled() {
                return Err(JobError::retryable("demo job was cancelled"));
            }
            let payload: ItemCreatedJob = serde_json::from_value(job.payload.clone())
                .map_err(|_| JobError::permanent("invalid item-created payload"))?;
            tracing::info!(
                message = "demo item-created job handled",
                job_id = %job.id,
                item_id = %payload.item_id,
            );
            Ok(())
        })
    }
}
```

The error you return decides what happens next:

- `JobError::permanent` fails the job now. Its `failure_reason` becomes `permanent`.
- `JobError::retryable` schedules another attempt with exponential backoff.
- `JobError::retryable_after` uses the delay you pass, for example a provider's `Retry-After`.

`max_attempts` still caps retries. A job that runs out ends with `failure_reason` set to
`attempts_exhausted`.

### Leases and side effects

A claim gives the worker a lease. If the process dies, the lease expires and another worker
reclaims the job, which consumes an attempt. Handlers therefore run more than once in some cases.
Send an idempotency key with every external side effect.

When the handler writes its result to the same database, commit the result and the job's
completion together:

1. Write the product rows in a transaction.
2. Call `complete_lease_in_transaction` with `cancellation.lease()`. It returns `false` if the
   lease is gone. Return `JobError::retryable` in that case.
3. Commit, call `cancellation.mark_completed_in_transaction()`, then return `Ok(())`.

`check_lease_held_in_transaction` checks the same condition without completing. Call it before
an intermediate write in a long job.

## Run the worker process

The generated `worker.rs` builds a `WorkerRunner` from a `PostgresJobStore`, the handler and a
`WorkerConfig`, and runs it under a `baukit_runtime::TaskSupervisor` with the process
`ShutdownToken`. On shutdown the runner stops claiming and drains running attempts within the
drain deadline. The ops listener serves readiness from `WorkerRunner::ready`, which runs the claim
query with `LIMIT 0` and so catches a missing or incompatible table.

The generated product reads these settings from the `worker` section of its config. The defaults
and checks live in `ProductConfig` in `backend/crates/<app>-bin/src/lib.rs`:

| Key | Default | Check |
|---|---|---|
| `worker.concurrency` | 5 | 1 to 64 |
| `worker.lease_duration_seconds` | 900 | above zero |
| `worker.job_timeout_seconds` | 600 | above zero, below the lease |
| `worker.poll_interval_milliseconds` | 250 | above zero |

Set them with environment variables, for example `MY_PRODUCT__WORKER__CONCURRENCY=10`. The
[baukit-config README](../../rust/crates/baukit-config/README.md#layering) explains the prefix and
the layering. The other `WorkerConfig` fields, such as the retry backoff bounds, keep the crate
defaults in the generated worker.

### Split job types across deployments

`WorkerRunner` passes its handler's `job_types()` on every claim. Two deployments with disjoint
handlers can share one outbox. Give each its own `WorkerConfig.queue` label. A job type that no
running handler declares stays pending with zero attempts, and the queue-age gauge keeps rising
until a worker for it starts.

## Schedule recurring work

`FixedUtcInterval` computes whole-second UTC slots anchored at the Unix epoch. There is no cron
parser and no scheduler. Each run enqueues the next slot before it reports success:

- `next_slot(current, now)` returns the first boundary after both the current slot and the clock.
  A run delayed from 12:00 to 12:47 on an hourly interval schedules 13:00.
- Use `slot.identifier()` as the idempotency key and `slot.starts_at()` as `run_after`. A duplicate
  delivery computes the same key and gets the existing row back.
- Put the product write, the next enqueue and the lease completion in one transaction.

The full handler body is in `templates/worker/docs/durable-jobs.md`, under "Enqueue a fixed
recurring slot". `FixedUtcJitter` spreads per-key jobs inside a slot without changing the slot
identifier. The tombstone purge in [data and sync](data-and-sync.md#purge-old-tombstones) uses
the same pattern.

## Cancel jobs with a domain change

`request_cancellation_by_key_in_transaction` and
`request_cancellation_by_key_prefix_in_transaction` cancel jobs in the caller's transaction. A
connection delete and the cancellation of its poll jobs then commit together. Pending jobs become
cancelled. Running jobs get a cancellation request, which the runner sees through
`JobCancellation`.

## Clean up terminal jobs

`PostgresJobStore::cleanup_terminal_jobs` takes a `TerminalJobCutoffs` with separate cutoffs for
succeeded, cancelled and failed jobs, plus a batch size up to
`MAX_TERMINAL_JOB_CLEANUP_BATCH_SIZE`. One call deletes one batch and returns counts per status.
It never touches pending or running rows. The product picks the retention and how often to call
it. The snippet is in `templates/worker/docs/durable-jobs.md`.

Two builders on the store keep jobs out of cleanup:

- `retain_kinds` keeps every terminal job of those kinds. Use it when another component owns their
  cleanup. A `baukit-suite` product passes `&baukit_suite::SUITE_JOB_TYPES`.
- `retain_failed_kinds` keeps only the failed ones, for repair.

## Identity deletion jobs

With `--auth`, the API process runs an `IdentityDeletionHandler` from `baukit-erasure` under its
own supervisor. The generated stores call
`retain_failed_kinds(&[baukit_erasure::IDENTITY_DELETE_JOB_TYPE])`, so a failed deletion stays in
the outbox for repair. Apply the same builder to any store you create for maintenance. Keep the
API deployed until pending identity deletion jobs finish. The [privacy guide](privacy.md) covers
erasure itself.

## Deploy the worker

The `baukit-app` chart has a `worker` block, off by default. The generated `deploy/values.yaml`
turns it on and points it at the worker image:

```yaml
worker:
  enabled: true
  image:
    repository: ghcr.io/<owner>/<product>-worker
    tag: "0.0.0-unset"
  command:
    - /app/worker
```

The chart defaults in `deploy/chart/baukit-app/values.yaml` set one replica, an ops port of 9090,
a 30 second termination grace period, and startup, liveness and readiness probes. Keep
`terminationGracePeriodSeconds` above the config's shutdown drain timeout, or Kubernetes kills
attempts mid-drain. The worker gets no Redis access unless you set `worker.redis.enabled: true`.
`baukit generate dockerfile` builds the `worker` stage when `capabilities.worker` is true. See
[deploying](deploying.md) for images and releases.

## Watch it

The runner emits the worker metric families from section 2.4 of the
[telemetry spec](../platform/telemetry-spec.md). Worker metrics label the handler type
`job_kind`, because Prometheus uses `job` for the scrape target. Group worker queries by
`job_kind`.

The shared alert rules in `deploy/observability/alerts/baukit.rules.yml` have these worker alerts:

| Alert | Fires when, for 10 minutes | Runbook |
|---|---|---|
| `BaukitWorkerFailureRateHigh` | more than 10% of runs fail for a `job_kind` | [worker-failure-rate-high](../runbooks/app/worker-failure-rate-high.md) |
| `BaukitWorkerRetryStorm` | more than one retry per second for a `job_kind` | [worker-retry-storm](../runbooks/app/worker-retry-storm.md) |
| `BaukitWorkerQueueOldestAgeHigh` | `worker_queue_oldest_age_seconds` above 300 | [worker-queue-oldest-age-high](../runbooks/app/worker-queue-oldest-age-high.md) |

A queue-age alert with no failures often means a job type that no deployed handler claims.

## Reference

- [baukit-jobs README](../../rust/crates/baukit-jobs/README.md)
- [baukit-runtime README](../../rust/crates/baukit-runtime/README.md)
- [baukit-config README](../../rust/crates/baukit-config/README.md)
- [baukit-erasure README](../../rust/crates/baukit-erasure/README.md)
- [Template durable-jobs notes](../../templates/worker/docs/durable-jobs.md)
- [Generated worker process](../../templates/worker/backend/crates/__app__-bin/src/bin/worker.rs)
- [baukit-app chart README](../../deploy/chart/baukit-app/README.md)
- [Telemetry spec](../platform/telemetry-spec.md)
- [Worker alert rules](../../deploy/observability/alerts/baukit.rules.yml)
- [ADR 0020: durable jobs, outbox and leases](../adr/0020-durable-jobs-outbox-and-leases.md)
- [ADR 0021: job ownership keys for erasure](../adr/0021-job-ownership-keys-for-erasure.md)
