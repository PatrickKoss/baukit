# Terminal-job cleanup evidence

- Source product files: `/home/patrick/projects/tiefgang/backend/crates/tiefgang-postgres/src/retention.rs`, especially `SweepTarget::JobOutboxCompleted`, `SweepTarget::JobOutboxFailed`, and their status predicates.
- Observed repeated glue: Tiefgang deletes Baukit-owned `job_outbox` statuses with product SQL. The old operation drains every matching batch in one call and can delay shutdown.
- Baukit owner: `baukit-jobs::PostgresJobStore`, which owns the PostgreSQL schema and lifecycle transitions.
- Public types and errors: `TerminalJobCutoffs`, `TerminalJobCleanupOutcome`, `MAX_TERMINAL_JOB_CLEANUP_BATCH_SIZE`, and `PostgresJobStore::cleanup_terminal_jobs`. Zero or oversized batches return `StoreError::InvalidInput`; database failures remain `StoreError::Database`.
- Product-owned inputs: three retention cutoffs, batch size, invocation schedule, shutdown deadline, metrics, and all product-table cleanup.
- Concurrency, failure, privacy, and cleanup cases: one total batch per call; locked rows are skipped; pending, running, and expired running rows survive; returned counts cover committed deletes only. The API accepts no payload data and returns no job content.
- Supported runtimes: PostgreSQL through the existing SQLx `PostgresJobStore` adapter.
- Product adoption change: a Tiefgang adoption pull request will replace the two `job_outbox` sweep targets and their status SQL in `backend/crates/tiefgang-postgres/src/retention.rs`. The rest of that product retention file remains.
