# 23. Durable job ownership

## Source product files

- `/home/patrick/projects/tiefgang/backend/crates/tiefgang-postgres/src/erasure.rs`
- `/home/patrick/projects/tiefgang/backend/crates/tiefgang-postgres/src/webhooks.rs`
- `/home/patrick/projects/eigenruhe/backend/crates/eigenruhe-services/src/connections.rs`
- `/home/patrick/projects/eigenruhe/backend/crates/eigenruhe-postgres/src/practice.rs`
- `/home/patrick/projects/eigenruhe/backend/crates/eigenruhe-postgres/src/profile.rs`

## Observed failure or repeated glue

Tiefgang deletes jobs by inspecting top-level `payload ->> 'auth_subject'` and `payload ->> 'user_id'`. Its current webhook job stores the subject at `event.user_id`, so that predicate misses the row. Eigenruhe's daily-context and hub-publish payloads carry `owner_id`, but profile erasure does not inspect or delete `job_outbox`; its conformance adapter reports zero background jobs.

## Baukit owner

`baukit-jobs` owns the outbox schema, owner-key validation, owned enqueue, lease-safe owner cancellation, bounded terminal deletion, and the supporting index.

## Public types and errors

The accepted sketch adds `JobOwnerKey`, `JobOwnerKeyError`, `OwnedJobStore`, `OwnedEnqueueError`, `OwnerCancellationOutcome`, and `OwnerDeletionOutcome`. Existing `NewJob`, `Job`, and `JobStore` remain source compatible. No implementation exists yet.

## Product-owned inputs

Products choose the identity-to-key mapping, which jobs are owner-scoped, erasure fence and retry policy, batch schedule, legacy backfill proof, external-effect idempotency, and product row deletion order.

## Concurrency, failure, privacy, and cleanup cases

Cancellation and deletion use bounded batches and `FOR UPDATE SKIP LOCKED`. Pending jobs cancel immediately; live running jobs receive a cooperative request; expired running jobs can cancel under lease rules; no running row is deleted. Enqueue and domain writes can share one transaction. Owner keys never enter metrics, logs, error text, or payload-bearing results. Erasure waits until no owner jobs remain, then deletes product data and writes its receipt.

## Supported runtimes

PostgreSQL through `PostgresJobStore`. Custom `JobStore` implementations remain valid and may opt into the additive `OwnedJobStore` trait.

## Index evidence

PostgreSQL 17 in Docker held 1,000,000 current-schema rows, including 800,000 owner-scoped rows across 8,000 owners. The partial `(owner_key, created_at, id)` index was 47,120,384 bytes, or 58.9 bytes per owned row. An owner selector changed from a sequential scan that rejected 999,945 rows to a bitmap scan of 100 owner entries. Measured warm-cache execution changed from 54.775 ms before the index to 0.119 ms after it; the comparable terminal selector took 0.111 ms.

## Product adoption change

Tiefgang can delete the payload-key job deletion in `backend/crates/tiefgang-postgres/src/erasure.rs`. Eigenruhe can replace the hard-coded zero in `PostgresProfileErasureAdapter::registered_background_job_count` and cover owner-scoped jobs in its erasure flow. Its global notification and retention jobs remain unowned.
