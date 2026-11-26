----------------------------- MODULE ErasureSaga -----------------------------
(* One subject's erasure: the erase transaction, the identity-deletion job
   with an inline attempt and a worker, a product request path, logins, and an
   IdP that may answer 404 for an account it still has. See README.md for the
   mapping to baukit-erasure. *)
EXTENDS Integers

CONSTANTS
    Accept404,    \* "any" (current) or "missingBody" (fixed): which 404 counts as deleted
    Requests,     \* "lockFree" (current, reject_fenced_subject) or "guarded" (fixed)
    MaxTime,      \* logical clock bound
    MaxAttempts   \* identity-deletion job attempts

ASSUME Accept404 \in {"any", "missingBody"} /\ Requests \in {"lockFree", "guarded"}

None == "none"
\* IdP answers to a DELETE: deleted now; 404 with the provider's "user not
\* found" body; 404 from a wrong base URL or a non-provider subject; 5xx.
Answers == {"ok", "404missing", "404other", "retryable"}

VARIABLES
    now,          \* logical clock (DB now())
    rows,         \* committed product rows, each tagged with its writer's auth_time
    account,      \* IdP account: "exists" or "deleted"
    op,           \* erasure_operations.state: None, "pending", "completed", "failed"
    erasedAt,     \* erasure_fences.erased_at, or -1
    job,          \* identity-deletion job: None, "pending", "running", "done", "failed"
    attempts,
    authAt,       \* the user's session auth_time (latest login)
    ePc,          \* erase request: "idle", "locked", "erased", "committed", "done"
    eDeleted,     \* rows the erase transaction's DELETE saw
    lockHeld,     \* subject advisory lock holder: None, "erase", "request"
    rPc,          \* product request: "idle", "checked"
    staleAdmit,   \* a product write committed for a session the fence rejects
    crashed       \* the erase handler crashed before commit once

vars == <<now, rows, account, op, erasedAt, job, attempts, authAt, ePc,
          eDeleted, lockHeld, rPc, staleAdmit, crashed>>

Init ==
    /\ now = 0
    /\ rows = {0}
    /\ account = "exists"
    /\ op = None
    /\ erasedAt = -1
    /\ job = None
    /\ attempts = 0
    /\ authAt = 0
    /\ ePc = "idle"
    /\ eDeleted = {}
    /\ lockHeld = None
    /\ rPc = "idle"
    /\ staleAdmit = FALSE
    /\ crashed = FALSE

\* REJECTED_SESSION_SQL: fenced if auth_time <= erased_at or an operation is
\* not completed.
Fenced(a) == erasedAt >= 0 /\ (a <= erasedAt \/ op # "completed")

\* The deleter's mapping of an IdP answer. Current: any 404 is success.
\* Fixed: only a 404 with the provider's "missing" body is.
Accepts(ans) ==
    \/ ans = "ok"
    \/ ans = "404missing"
    \/ ans = "404other" /\ Accept404 = "any"

\* Which answers the IdP can give right now.
Possible(ans) ==
    CASE ans = "ok"         -> account = "exists"
      [] ans = "404missing" -> account = "deleted"
      [] OTHER              -> TRUE

Tick ==
    /\ now < MaxTime
    /\ now' = now + 1
    /\ UNCHANGED <<rows, account, op, erasedAt, job, attempts, authAt, ePc,
                   eDeleted, lockHeld, rPc, staleAdmit, crashed>>

Login ==
    /\ authAt < now
    /\ authAt' = now
    /\ UNCHANGED <<now, rows, account, op, erasedAt, job, attempts, ePc,
                   eDeleted, lockHeld, rPc, staleAdmit, crashed>>

(* ---- Erase: one transaction, then an inline attempt ---- *)

\* lock(key) + replay check + guard_subject (takes the subject lock).
EraseLock ==
    /\ ePc = "idle" /\ op = None
    /\ lockHeld = None
    /\ ~Fenced(authAt)
    /\ lockHeld' = "erase"
    /\ ePc' = "locked"
    /\ UNCHANGED <<now, rows, account, op, erasedAt, job, attempts, authAt,
                   eDeleted, rPc, staleAdmit, crashed>>

\* product.erase: DELETE sees the rows committed now. Rows a concurrent
\* transaction commits later are not deleted.
EraseProduct ==
    /\ ePc = "locked"
    /\ eDeleted' = rows
    /\ ePc' = "erased"
    /\ UNCHANGED <<now, rows, account, op, erasedAt, job, attempts, authAt,
                   lockHeld, rPc, staleAdmit, crashed>>

\* operation row + fence + enqueue job, commit.
EraseCommit ==
    /\ ePc = "erased"
    /\ rows' = rows \ eDeleted
    /\ op' = "pending"
    /\ erasedAt' = now
    /\ job' = "pending"
    /\ lockHeld' = None
    /\ ePc' = "committed"
    /\ UNCHANGED <<now, account, attempts, authAt, eDeleted, rPc, staleAdmit, crashed>>

\* A crash or error before commit rolls everything back.
EraseCrash ==
    /\ ePc \in {"locked", "erased"}
    /\ ~crashed
    /\ crashed' = TRUE
    /\ ePc' = "idle"
    /\ lockHeld' = None
    /\ UNCHANGED <<now, rows, account, op, erasedAt, job, attempts, authAt,
                   eDeleted, rPc, staleAdmit>>

\* inline: only an unclaimed job; success completes the operation and deletes
\* the job in one transaction. Any failure leaves the job to the worker.
Inline(ans) ==
    /\ ePc = "committed"
    /\ ePc' = "done"
    /\ IF job = "pending" /\ Possible(ans)
       THEN /\ account' = IF ans = "ok" THEN "deleted" ELSE account
            /\ IF Accepts(ans)
               THEN op' = "completed" /\ job' = "done"
               ELSE UNCHANGED <<op, job>>
       ELSE UNCHANGED <<account, op, job>>
    /\ UNCHANGED <<now, rows, erasedAt, attempts, authAt, eDeleted, lockHeld,
                   rPc, staleAdmit, crashed>>

(* ---- Worker (IdentityDeletionHandler on baukit-jobs) ---- *)

Claim ==
    /\ job = "pending" /\ attempts < MaxAttempts
    /\ job' = "running"
    /\ attempts' = attempts + 1
    /\ UNCHANGED <<now, rows, account, op, erasedAt, authAt, ePc, eDeleted,
                   lockHeld, rPc, staleAdmit, crashed>>

\* delete_account, then complete_in_transaction + store.complete + delete job,
\* or a retryable/permanent failure. The 0001 trigger marks the operation
\* failed when the job fails.
WorkerDelete(ans) ==
    /\ job = "running"
    /\ Possible(ans)
    /\ account' = IF ans = "ok" THEN "deleted" ELSE account
    /\ IF Accepts(ans)
       THEN op' = "completed" /\ job' = "done"
       ELSE IF attempts < MaxAttempts
            THEN job' = "pending" /\ UNCHANGED op
            ELSE job' = "failed" /\ op' = "failed"
    /\ UNCHANGED <<now, rows, erasedAt, attempts, authAt, ePc, eDeleted,
                   lockHeld, rPc, staleAdmit, crashed>>

\* The worker stalls and its lease expires: claim's sweep fails an exhausted
\* job, otherwise the job is claimable again.
LeaseExpiry ==
    /\ job = "running"
    /\ IF attempts >= MaxAttempts
       THEN job' = "failed" /\ op' = "failed"
       ELSE job' = "pending" /\ UNCHANGED op
    /\ UNCHANGED <<now, rows, account, erasedAt, attempts, authAt, ePc,
                   eDeleted, lockHeld, rPc, staleAdmit, crashed>>

(* ---- Product request that writes for the subject ---- *)

Write ==
    /\ rows' = rows \cup {authAt}
    /\ staleAdmit' = (staleAdmit \/ Fenced(authAt))

\* Lock-free: reject_fenced_subject reads the fence, then a later transaction
\* inserts (get-or-create profile, push registration).
RequestCheck ==
    /\ Requests = "lockFree"
    /\ rPc = "idle"
    /\ ~Fenced(authAt)
    /\ rPc' = "checked"
    /\ UNCHANGED <<now, rows, account, op, erasedAt, job, attempts, authAt,
                   ePc, eDeleted, lockHeld, staleAdmit, crashed>>

RequestWrite ==
    /\ rPc = "checked"
    /\ Write
    /\ rPc' = "idle"
    /\ UNCHANGED <<now, account, op, erasedAt, job, attempts, authAt, ePc,
                   eDeleted, lockHeld, crashed>>

\* Guarded: guard_subject and the insert in one transaction under the
\* subject lock. Modelled as one step taken while nobody holds the lock.
RequestGuarded ==
    /\ Requests = "guarded"
    /\ rPc = "idle"
    /\ lockHeld = None
    /\ ~Fenced(authAt)
    /\ Write
    /\ UNCHANGED <<now, account, op, erasedAt, job, attempts, authAt, ePc,
                   eDeleted, lockHeld, rPc, crashed>>

Next ==
    \/ Tick \/ Login
    \/ EraseLock \/ EraseProduct \/ EraseCommit \/ EraseCrash
    \/ \E a \in Answers : Inline(a) \/ WorkerDelete(a)
    \/ Claim \/ LeaseExpiry
    \/ RequestCheck \/ RequestWrite \/ RequestGuarded

\* The job runner keeps claiming and running the job; the erase handler
\* finishes its steps. Lease expiry and crashes are not fair.
Fairness ==
    /\ WF_vars(Claim)
    /\ WF_vars(\E a \in Answers : WorkerDelete(a))
    /\ WF_vars(\E a \in Answers : Inline(a))

Spec == Init /\ [][Next]_vars
FairSpec == Spec /\ Fairness

(* Invariants *)

\* Rows written by a session that logged in after the fence belong to a new
\* profile and are allowed.
CompletedMeansErased ==
    op = "completed" =>
        /\ \A a \in rows : a > erasedAt
        /\ account = "deleted"

NoStaleAdmission == ~staleAdmit

(* Liveness: every operation ends completed or failed. *)
OperationEnds == (op = "pending") ~> (op \in {"completed", "failed"})

=============================================================================
