# M-06 ErasureSaga

One subject's erasure with identity deletion: the erase transaction and its inline attempt, the job worker,
a product request that writes for the subject, logins, crashes, and an IdP that may answer 404 for an
account it still has. Settles F-40 and F-42.

## What maps to what

| Model | Code (`rust/crates/baukit-erasure/src`) |
|---|---|
| `EraseLock` | `erase`: key lock, replay check, `guard_subject` (subject advisory lock, `REJECTED_SESSION_SQL`) |
| `EraseProduct` | `product.erase`: the DELETE sees rows committed at that point |
| `EraseCommit` | operation row, fence (`erased_at = now()`), job enqueue, commit |
| `EraseCrash` | any error or crash before commit (rollback) |
| `Inline` | `inline`: only an unclaimed job; success completes the operation and deletes the job |
| `Claim`, `WorkerDelete`, `LeaseExpiry` | `worker.rs` `IdentityDeletionHandler` on baukit-jobs; the 0001 trigger marks the operation failed when the job fails |
| `Accepts` | `api.rs:63` and `keycloak.rs:139`: today any 404 is success |
| `RequestCheck`, `RequestWrite` | `reject_fenced_subject` then a separate insert (lock-free) |
| `RequestGuarded` | `guard_subject` and the insert in one transaction under the subject lock |
| `Login`, `Tick` | a new IdP login (`auth_time`), DB time passing |

Fixed design: `Accept404 = "missingBody"` (404 counts only with the provider's "user not found" body, K-23)
and `Requests = "guarded"` (product writes run `guard_subject` in their own transaction). Not modelled:
`OperatorFence` (`fence_as_operator_in_transaction`), which adds a fence without product erasure.

## Properties

- `CompletedMeansErased`: a completed operation implies the IdP account is gone and no product row from a
  session at or before `erased_at` survives. Rows from a login after the fence belong to a new profile.
- `NoStaleAdmission`: no product write commits for a session the fence rejects.
- `OperationEnds` (liveness): `op = pending ~> op \in {completed, failed}`, under weak fairness on claim,
  worker deletion and the inline attempt. Lease expiry and crashes are unfair; the bounded attempt count
  makes a persistent failure end in `failed`.

## Configs

| Config | Expect |
|---|---|
| `ErasureSaga.cfg` | pass |
| `ErasureSaga_current.cfg` | violation of `CompletedMeansErased` (F-40) |
| `ErasureSaga_lockFree_current.cfg` | violation of `NoStaleAdmission` (F-42) |

`MaxTime = 2`, `MaxAttempts = 2`.

## Counterexamples on today's code

F-40: the erase locks, deletes the product rows and commits a pending operation and the fence. The inline
attempt calls the IdP, which answers 404 because the base URL is wrong. The deleter treats it as success,
the operation goes `completed`, and the IdP account still exists.

F-42: the erase takes the subject lock and deletes the product rows. A product request passes
`reject_fenced_subject`, which reads the fence without the lock. The erase commits the fence. The request
then inserts its row, which survives the erasure, and the session it came from is one the fence rejects.

## Paired tests

P-14 (erasure service state machine on Postgres, concurrent product writes through the template helper),
K-22 (`fence_rejects`, `replay_outcome`), K-23 (`deletion_status_outcome`), fuzz Z-12.
