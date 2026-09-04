# Durable job maintenance and recurrence

The generated worker uses Baukit's `job_outbox` state machine. Applications
choose retention periods and recurring-job intervals.

## Delete old terminal jobs

Build one `TerminalJobCutoffs` value from application configuration and call
`PostgresJobStore::cleanup_terminal_jobs`. The batch size must be nonzero and
no larger than `MAX_TERMINAL_JOB_CLEANUP_BATCH_SIZE`.

One call deletes one batch across succeeded, cancelled, and failed jobs. It
never deletes pending or running rows, including expired running leases. Run
another batch only while the application's maintenance deadline and shutdown
state permit it. Keep product-table cleanup in the product repository.

```rust
let deleted = jobs
    .cleanup_terminal_jobs(
        TerminalJobCutoffs {
            succeeded_before,
            cancelled_before,
            failed_before,
        },
        cleanup_batch_size,
    )
    .await?;
tracing::info!(
    succeeded = deleted.succeeded,
    cancelled = deleted.cancelled,
    failed = deleted.failed,
    "terminal job cleanup finished"
);
```

## Enqueue a fixed recurring slot

Store the UTC slot in the job payload. Validate it by comparing it with
`interval.slot_at(payload.slot)`. Calculate the next slot from both the current
payload slot and the handler clock. This skips missed boundaries and continues
forward if the wall clock moved backward.

```rust
let interval = FixedUtcInterval::new(Duration::from_secs(60 * 60))
    .map_err(|_| JobError::permanent("invalid_recurring_interval"))?;
let current = interval
    .slot_at(payload.slot)
    .map_err(|_| JobError::permanent("invalid_recurring_slot"))?;
if current.starts_at() != payload.slot {
    return Err(JobError::permanent("invalid_recurring_slot"));
}
let next = interval
    .next_slot(current, clock.now())
    .map_err(|_| JobError::permanent("invalid_recurring_slot"))?;

let mut transaction = pool.begin().await.map_err(store_job_error)?;
write_product_result(&mut transaction).await?;
jobs
    .enqueue_in_transaction(
        &mut transaction,
        NewJob::new(JOB_TYPE, payload_for(next.starts_at()), MAX_ATTEMPTS)
            .run_after(next.starts_at())
            .idempotency_key(next.identifier()),
    )
    .await
    .map_err(store_job_error)?;
let completed = jobs
    .complete_in_transaction(
        &mut transaction,
        job.id,
        cancellation.worker_id(),
        clock.now(),
    )
    .await
    .map_err(store_job_error)?;
if !completed {
    return Err(JobError::retryable("job_lease_lost"));
}
transaction.commit().await.map_err(store_job_error)?;
cancellation.mark_completed_in_transaction();
Ok(())
```

The transaction keeps the product write, next enqueue, and current completion
together. If enqueue fails, return an error and leave the current attempt
unfinished. Duplicate delivery and process restart reuse the same idempotency
key, so they do not create a second next-slot row.
