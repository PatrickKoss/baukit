# Recurring-job slot evidence

- Source product files: `/home/patrick/projects/eigenruhe/backend/crates/eigenruhe-worker/src/lib.rs` and `/home/patrick/projects/eigenruhe/backend/crates/eigenruhe-worker/src/retention.rs`.
- Observed repeated glue: Eigenruhe's notification and retention handlers each round UTC time, add one interval, build a timestamp idempotency key, and self-enqueue.
- Baukit owner: the pure `baukit-jobs` fixed UTC slot module. Existing job-store methods continue to own durable and transactional enqueue behavior.
- Public types and errors: `FixedUtcInterval`, `FixedUtcSlot`, and `FixedUtcIntervalError`. The error distinguishes zero, fractional-second, and out-of-range intervals or slots.
- Product-owned inputs: interval, seed job, job type, payload, maximum attempts, handler clock, catch-up choice, and stop policy.
- Concurrency, failure, privacy, and cleanup cases: delayed jobs choose the next wall-clock boundary; missed slots are skipped; backward clocks still advance past the current slot; duplicate delivery and restart reuse one identifier. Enqueue failure remains an explicit handler error. Identifiers contain only a UTC timestamp.
- Supported runtimes: any Rust runtime supported by `baukit-jobs`; slot calculation is pure and does not require Tokio or PostgreSQL.
- Product adoption change: an Eigenruhe adoption pull request will delete `notification_slot`, `notification_job_key`, `retention_slot`, and `retention_job_key` from `backend/crates/eigenruhe-worker/src/lib.rs`, then use the shared helper in both handlers.
