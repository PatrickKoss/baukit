# ADR 0021: Jobs carry an optional owner key so erasure can cancel and delete them

## Status
Proposed, 2026-11-27.

## Context
Erasure must remove a user's pending and finished jobs. Today a product can only find them by reading fields out of JSON payloads, and payload shapes differ per job type. A consumer product's erasure predicate missed jobs whose subject sat at a nested path.

`baukit-jobs` owns `job_outbox`, so it is the place to record ownership without parsing payloads.

## Decision
Proposed: add an opaque `owner_key TEXT` column with a 1 to 500 byte check and a partial index on `(owner_key, created_at, id)`. Add a validated `JobOwnerKey`, an additive `OwnedJobStore: JobStore` trait with `enqueue_owned` and `request_owner_cancellation`, and on `PostgresJobStore` the methods `enqueue_owned_in_transaction`, `delete_terminal_jobs_by_owner` and `owner_jobs_remain`. Batches are bounded and use `FOR UPDATE SKIP LOCKED`. Existing rows stay NULL and baukit never infers ownership from payloads. `NewJob`, `Job` and the required `JobStore` methods stay unchanged.

Erasure order: commit the product's erasure fence, cancel owned work in batches, wait until running jobs acknowledge or lose their leases, delete terminal rows, then erase product rows.

What the code does today, at this commit: none of the owner key exists. No `owner_key` column, type, trait or method is in `rust/crates/baukit-jobs`. The parts the design builds on do exist: per-job `request_cancellation`, `cancellation_requested` polling in the runner, attempt-bound leases from [ADR 0020](0020-durable-jobs-outbox-and-leases.md), and `baukit_erasure::WriteFence` as a fence the erasure order can use.

## Enforced by
Nothing. The change would ship with Postgres tests for owner isolation, mismatched idempotency ownership, batch limits, locked-row skipping and cancellation races.

## Consequences
Products would get owner-scoped job erasure without payload parsing and pay for an extra index. Until then they delete jobs by product SQL over payloads.

## History
None. The design dates from before this ADR and is unimplemented.
