# Job leases are bound to the attempt

`JobStore::complete`, `record_failure` and `cancel` now take a `&JobLease`
(`job_id`, `worker_id`, `attempt`) instead of a job id and worker id. Before,
two runner slots with one worker id could act on each other's attempt: after a
lease expired and a sibling slot reclaimed the job, the stale slot could still
complete it. Every lease predicate now also requires `attempts = $attempt`.

Build the lease from the claimed job with `JobLease::of(&job, worker_id)`, or
from `JobCancellation::lease()` inside a handler.

| Old call | New call |
| --- | --- |
| `store.complete(job_id, worker, now)` | `store.complete(&lease, now)` |
| `store.record_failure(job_id, worker, ...)` | `store.record_failure(&lease, ...)` |
| `store.cancel(job_id, worker, now)` | `store.cancel(&lease, now)` |
| `complete_in_transaction(tx, job_id, worker, now)` | `complete_lease_in_transaction(tx, &lease, now)` |
| `check_lease_in_transaction(tx, job_id, worker, now)` | `check_lease_held_in_transaction(tx, &lease, now)` |

The two old transaction methods still compile but ignore the attempt. Move
off them. Custom `JobStore` implementations must take the new signatures.

The runner now logs a failed attempt and keeps polling instead of stopping,
and caps a handler's `retry_after` at `retry_max`.
