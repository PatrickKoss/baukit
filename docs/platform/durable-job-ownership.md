# Durable job ownership

**Status:** Accepted design. No runtime or migration implements it yet.
**Applies to:** Optional erasure or account-scoped operations on the `baukit-jobs` PostgreSQL outbox.
**Related:** [Product profile erasure](./product-profile-erasure-contract.md) and the [crate README](../../rust/crates/baukit-jobs/README.md).

## Problem and boundary

Tiefgang tries to delete personal jobs by reading top-level `auth_subject` and `user_id` fields from JSON payloads. Its current `EventsDeliverJobV1` stores the subject at `event.user_id`, so the erasure predicate does not match that job shape. A payload rename or nesting change can leave a row behind. Eigenruhe has the same ownership fact in two job payloads but does not register those jobs with its erasure adapter.

`baukit-jobs` should own an optional `owner_key` because it already owns `job_outbox`. The key is opaque text. Baukit does not parse it, prescribe an identity format, join it to a product table, or expose it as a metric label. Products decide which jobs have an owner and how an application identity maps to the key. Global maintenance jobs keep a null key.

Use `owner_key`, not `partition_key`. The intended operation is lifecycle control for one owner. The name `partition_key` would also imply queue routing or database partitioning, which this design does not provide.

This work stays separate from terminal-job cleanup. Retention deletes old terminal rows by status and cutoff. Owner erasure cancels current work and removes every terminal row for one opaque key, regardless of age.

## Additive schema migration

Products would copy a new ordered migration after the current Baukit job migrations:

```sql
ALTER TABLE job_outbox
    ADD COLUMN owner_key TEXT;

ALTER TABLE job_outbox
    ADD CONSTRAINT job_outbox_owner_key_check CHECK (
        owner_key IS NULL OR octet_length(owner_key) BETWEEN 1 AND 500
    ) NOT VALID;

ALTER TABLE job_outbox
    VALIDATE CONSTRAINT job_outbox_owner_key_check;
```

The API preserves the supplied UTF-8 text exactly. It does not trim, case-fold, hash, or otherwise normalize it. The check rejects empty values and values over 500 bytes. Products should normally use an internal opaque identifier rather than an email address or another display value.

Build the supporting index separately because `CREATE INDEX CONCURRENTLY` cannot run inside a transaction block:

```sql
CREATE INDEX CONCURRENTLY job_outbox_owner_idx
    ON job_outbox (owner_key, created_at, id)
    WHERE owner_key IS NOT NULL;
```

The partial index omits global jobs. It also omits `status`, so normal claim, retry, and completion transitions do not rewrite the owner index. Owner operations filter lifecycle state after the index has limited the scan to one owner.

Existing rows remain null. Baukit must not infer ownership from payload JSON. A product may backfill a row only when product SQL can prove the mapping. Rows that cannot be proven remain on the old unowned lifecycle and need a product-specific migration sweep or ordinary terminal cleanup.

Deploy the column, constraint, and index before code calls an owned method. An old binary continues to enqueue null keys after the migration and can claim rows that have keys because claim SQL does not filter by owner. Rolling back to that binary preserves stored keys but temporarily removes the owner-erasure operation. The new release must keep the existing unowned paths for products that do not opt in.

## Public API sketch

The implementation should add a validated `JobOwnerKey` newtype and leave `NewJob`, `Job`, `EnqueueOutcome`, and the required methods on `JobStore` unchanged. Adding a field to either public struct would break external struct literals. Requiring new `JobStore` methods would break custom stores.

An additive `OwnedJobStore: JobStore` trait should expose owned enqueue and cancellation. `PostgresJobStore` should implement that trait and add the transaction form:

```rust
let owner = JobOwnerKey::try_from(user_id.to_string())?;
let outcome = store
    .enqueue_owned_in_transaction(&mut transaction, job, &owner)
    .await?;
```

The proposed public additions are:

- `JobOwnerKey` and `JobOwnerKeyError::{Empty, TooLong}`;
- `OwnedJobStore::enqueue_owned`;
- `PostgresJobStore::enqueue_owned_in_transaction`;
- `OwnedJobStore::request_owner_cancellation`;
- `PostgresJobStore::delete_terminal_jobs_by_owner`;
- `PostgresJobStore::owner_jobs_remain`;
- `OwnerCancellationOutcome` and `OwnerDeletionOutcome`; and
- `OwnedEnqueueError::{OwnerMismatch, Store}`.

Both mutation methods accept a batch size from 1 through a shared maximum of 10,000. They use `FOR UPDATE SKIP LOCKED`, change at most that many rows, and return committed counts. `owner_jobs_remain` is a bounded existence probe, not a row count.

The unowned `enqueue` methods continue to write null. On an owned idempotency conflict, the stored and supplied owner keys must match. A null or different stored key returns `OwnedEnqueueError::OwnerMismatch` without returning either key. The library must not attach a new owner to a legacy row merely because its idempotency key collided.

## Cancellation and running jobs

Owner cancellation applies these transitions:

- Pending rows become `cancelled` immediately.
- Running rows with a live lease receive `cancel_requested_at` and remain running.
- Running rows whose lease has expired become `cancelled` under the same lease and row-lock rules used by claim recovery.
- Terminal rows do not change.

The owner operation never deletes a running row. A worker with a live lease can already have sent an external request when cancellation commits. Destination idempotency and any provider-side revocation remain product responsibilities.

The current runner polls `cancellation_requested`, prevents normal completion after the request commits, and records `cancelled` when the handler yields. A handler that committed `complete_in_transaction` first has already won the race; owner cleanup sees a terminal succeeded row and may delete it. If no worker is alive, a later owner-cancellation call can cancel the row after lease expiry.

Locked rows are skipped, so an underfilled batch does not prove completion. Callers repeat cancellation and terminal deletion, then use `owner_jobs_remain`. A true result means erasure must wait and retry.

## Erasure ordering

An application needs a product-owned erasure fence. For example, it can mark the account as erasing while holding the same product identity lock that owner-scoped enqueue paths check. `enqueue_owned_in_transaction` makes the outbox insert atomic with a domain write, but it cannot stop a producer that ignores the product's erasure state.

Use this order:

1. Commit the product erasure fence so new owner-scoped writes and enqueues fail.
2. Repeat bounded owner cancellation until no actionable pending or running rows remain.
3. Wait for live running jobs to acknowledge cancellation or lose their leases. Do not report erasure complete while `owner_jobs_remain` is true.
4. Repeat bounded terminal deletion until the owner has no job rows.
5. Erase product rows and credentials in the product's foreign-key-safe order, replace or remove the identity, and write the idempotent erasure receipt.

A crash resumes from the fence and receipt state. The product chooses the retry schedule and user-visible pending response. It must not bypass the wait by deleting a live running job.

## Product mappings

### Tiefgang

`backend/crates/tiefgang-postgres/src/webhooks.rs` has the internal user UUID while it enqueues `events.deliver` in the event transaction. It would pass that UUID as `owner_key` through `enqueue_owned_in_transaction`. The event payload can keep its product-defined subject because the handler needs it; erasure no longer depends on that JSON shape.

During adoption, Tiefgang can backfill provable legacy rows by joining `payload -> 'event' ->> 'user_id'` to `users.auth_subject`, then retain a one-time fallback sweep for ambiguous null rows. Its erasure flow can then replace the incorrect top-level `payload ->> 'auth_subject'` and `payload ->> 'user_id'` deletion in `backend/crates/tiefgang-postgres/src/erasure.rs` with the bounded owner operations.

### Eigenruhe

`DailyContextPullPayloadV1` and `HubSessionCompletedPublishPayloadV1` both carry `owner_id`. Their producers in `backend/crates/eigenruhe-services/src/connections.rs` and `backend/crates/eigenruhe-postgres/src/practice.rs` would set that UUID as `owner_key`. The practice path proves the transaction form is needed.

`notifications.evaluate` and `retention.purge.v1` scan product state globally, so they remain unowned. Eigenruhe's `PostgresProfileErasureAdapter` currently returns zero from `registered_background_job_count` and does not delete `job_outbox`. Adoption would register the owner-scoped jobs, run the bounded owner flow before deleting `external_connections` and other owner rows, and keep global jobs untouched.

## Index measurement

The index sketch was measured in `postgres:17-alpine` using the current `0001_baukit_jobs.sql` and `0002_baukit_jobs_failure_reason.sql` schemas plus the proposed column and check. The table held 1,000,000 rows, 800,000 with an owner key. There were 8,000 non-null owners with 100 rows each. Statuses per owner were 50 pending, 5 running, 35 succeeded, 5 failed, and 5 cancelled.

The heap was 196 MB and the three existing indexes used 70 MB. The proposed partial index used 47,120,384 bytes, reported as 45 MB, or 58.9 bytes per indexed row. It added about 24 percent of the heap size and 67 percent of the existing index footprint. `CREATE INDEX CONCURRENTLY` took 521 ms in the local Docker run. That build time is hardware and cache dependent.

Before the index, `EXPLAIN (ANALYZE, BUFFERS)` for a 100-row owner cancellation selector used a sequential scan, rejected 999,945 rows, touched 25,133 heap buffers, and took 54.775 ms. With the index it used a bitmap index scan over 100 owner entries, touched 104 buffers before row locking, and took 0.119 ms on a warm cache. The terminal selector used the same 100-entry bitmap scan and took 0.111 ms. Products should repeat the size and build measurement against a production-like restore before applying the concurrent index.

## Compatibility and required tests

Implementation tests must cover a migration containing existing null rows, unowned enqueue, owned enqueue in and outside a caller transaction, rollback, exact key bounds, owner isolation, mismatched idempotency ownership, batch limits, locked-row skipping, pending cancellation, cooperative running cancellation, lease expiry, terminal deletion, and a concurrent completion race. Telemetry, errors, debug output, and returned outcomes must contain no owner key or payload value.

The product compatibility tests must keep legacy null jobs claimable and cleanable. Tiefgang and Eigenruhe must each pass profile-erasure conformance with an owned pending job, an owned running job, another owner's job, and a global null-owned job.

## Decision

Implement `owner_key` as a separate opt-in `baukit-jobs` change with the migration, index, additive traits, and bounded operations above. Do not combine it with terminal cleanup. The implementation is unlocked when Tiefgang and Eigenruhe each commit to the mappings above, and a production-like index review accepts the measured storage and build cost.
