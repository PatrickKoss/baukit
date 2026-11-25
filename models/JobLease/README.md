# M-02 JobLease

One `job_outbox` row, three runner slots (two share worker id `w1`, one belongs to `w2`), lease expiry and
cancellation. Settles F-05.

## What maps to what

| Model | Code (`rust/crates/baukit-jobs/src/store.rs`) |
|---|---|
| `Claim` | `claim`: the candidate CTE and `attempts = attempts + 1`, `locked_by`, `locked_until` |
| `Sweep` | the two sweeps at the top of `claim` (expired and cancel requested, expired and exhausted) |
| `Expire` | time passing `locked_until` while a handler stalls |
| `Owns` | `owned_lease_predicate`: `status = 'running' AND locked_by = $2 AND locked_until > $3` |
| `Complete`, `Fail`, `Cancel` | `complete`/`finish_on`, `record_failure`, `cancel`; a failed predicate is `LeaseLost` in `runner.rs` |
| `RequestCancel` | `request_cancellation` (`cancellation_assignments`) |

Fixed design: `Owns` also requires `attempts = <the slot's attempt>`, the attempt number in every terminal
write predicate.

## Properties

- `WriteByLatestAttempt`: every owned write came from the attempt that holds the lease ("succeeded implies
  completed by the latest attempt", applied to every terminal write).
- `OneTerminalWritePerLease`: at most one terminal write per attempt number.
- `AttemptsBounded`: `attempts <= max_attempts`.
- `PendingEventuallyClaimed` (liveness): `status = pending ~> status # pending`, under weak fairness on
  "some idle slot claims" and on each slot finishing its attempt. Expiry and cancellation are not fair.

## Configs

| Config | Expect |
|---|---|
| `JobLease.cfg` | pass (fixed predicate) |
| `JobLease_current.cfg` | violation of `WriteByLatestAttempt` |

`MaxAttempts = 3`.

## Counterexample on today's code

Slot s1 (worker w1) claims the job as attempt 1. Its lease expires while the handler stalls. Slot s2, same
worker id, reclaims it as attempt 2. s1 wakes up and calls `complete`: `locked_by = w1` and the lease is
live (s2's lease), so the write succeeds and the job is marked succeeded while attempt 2 still runs.

## Paired tests

P-10 (jobs state machine on Postgres, several slots sharing one worker id) and K-17 (`lease_predicate_holds`
mirror of the SQL predicate).
