------------------------------ MODULE JobLease ------------------------------
(* One job_outbox row, runner slots that share a worker id, lease expiry,
   cancellation. See README.md for the mapping to baukit-jobs. *)
EXTENDS Naturals, FiniteSets

CONSTANTS
    Slots,       \* runner concurrency slots; "s3" belongs to a second runner
    MaxAttempts,
    Design       \* "current": lease predicate is worker id + expiry
                 \* "fixed":   the attempt number is part of every terminal write

ASSUME Design \in {"current", "fixed"} /\ MaxAttempts \in Nat \ {0}

\* Slots of one runner share its worker id.
WorkerOf == [s \in Slots |-> IF s = "s3" THEN "w2" ELSE "w1"]

None == "none"
Terminal == {"succeeded", "failed", "cancelled"}

VARIABLES
    status,     \* pending, running, succeeded, failed, cancelled
    attempts,   \* job_outbox.attempts
    lockedBy,   \* worker id or None
    leaseLive,  \* locked_until > now
    cancelReq,  \* cancel_requested_at IS NOT NULL
    held,       \* slot -> attempt number it is running, 0 when idle
    writes      \* owned writes so far: set of [slot, attempt, latest, kind]

vars == <<status, attempts, lockedBy, leaseLive, cancelReq, held, writes>>

TypeOK ==
    /\ status \in {"pending", "running"} \cup Terminal
    /\ attempts \in 0..MaxAttempts
    /\ held \in [Slots -> 0..MaxAttempts]

Init ==
    /\ status = "pending"
    /\ attempts = 0
    /\ lockedBy = None
    /\ leaseLive = FALSE
    /\ cancelReq = FALSE
    /\ held = [s \in Slots |-> 0]
    /\ writes = {}

\* claim: the candidate CTE plus UPDATE ... attempts = attempts + 1.
Claimable ==
    /\ attempts < MaxAttempts
    /\ ~cancelReq
    /\ \/ status = "pending"
       \/ status = "running" /\ ~leaseLive

Claim(s) ==
    /\ held[s] = 0
    /\ Claimable
    /\ status' = "running"
    /\ attempts' = attempts + 1
    /\ lockedBy' = WorkerOf[s]
    /\ leaseLive' = TRUE
    /\ held' = [held EXCEPT ![s] = attempts + 1]
    /\ UNCHANGED <<cancelReq, writes>>

\* The two sweeps at the top of claim: expired + cancel requested, expired + exhausted.
Sweep ==
    /\ status = "running" /\ ~leaseLive
    /\ \/ cancelReq /\ status' = "cancelled"
       \/ ~cancelReq /\ attempts >= MaxAttempts /\ status' = "failed"
    /\ lockedBy' = None
    /\ UNCHANGED <<attempts, leaseLive, cancelReq, held, writes>>

\* Time passes past locked_until (a stalled handler, a GC pause).
Expire ==
    /\ status = "running" /\ leaseLive
    /\ leaseLive' = FALSE
    /\ UNCHANGED <<status, attempts, lockedBy, cancelReq, held, writes>>

\* owned_lease_predicate, plus the attempt number in the fixed design.
Owns(s) ==
    /\ status = "running"
    /\ lockedBy = WorkerOf[s]
    /\ leaseLive
    /\ Design = "fixed" => attempts = held[s]

Record(s, kind) == writes' = writes \cup
    {[slot |-> s, attempt |-> held[s], latest |-> attempts, kind |-> kind]}

\* complete / complete_in_transaction. A failed predicate is LeaseLost: the
\* slot gives up either way.
Complete(s) ==
    /\ held[s] > 0
    /\ IF Owns(s)
       THEN /\ status' = "succeeded" /\ lockedBy' = None
            /\ cancelReq' = FALSE
            /\ Record(s, "succeeded")
       ELSE UNCHANGED <<status, lockedBy, cancelReq, writes>>
    /\ held' = [held EXCEPT ![s] = 0]
    /\ UNCHANGED <<attempts, leaseLive>>

\* record_failure with retry: pending while attempts remain, else failed.
Fail(s, retry) ==
    /\ held[s] > 0
    /\ IF Owns(s)
       THEN /\ status' = IF retry /\ attempts < MaxAttempts THEN "pending" ELSE "failed"
            /\ lockedBy' = None
            /\ cancelReq' = FALSE
            /\ Record(s, IF status' = "failed" THEN "failed" ELSE "retry")
       ELSE UNCHANGED <<status, lockedBy, cancelReq, writes>>
    /\ held' = [held EXCEPT ![s] = 0]
    /\ UNCHANGED <<attempts, leaseLive>>

\* cancel: the handler saw cancellation_requested and stops.
Cancel(s) ==
    /\ held[s] > 0
    /\ cancelReq
    /\ IF Owns(s)
       THEN /\ status' = "cancelled" /\ lockedBy' = None /\ cancelReq' = FALSE
            /\ Record(s, "cancelled")
       ELSE UNCHANGED <<status, lockedBy, cancelReq, writes>>
    /\ held' = [held EXCEPT ![s] = 0]
    /\ UNCHANGED <<attempts, leaseLive>>

\* request_cancellation (cancellation_assignments).
RequestCancel ==
    /\ status \in {"pending", "running"}
    /\ ~cancelReq
    /\ IF status = "pending"
       THEN status' = "cancelled" /\ cancelReq' = FALSE /\ lockedBy' = None
       ELSE cancelReq' = TRUE /\ UNCHANGED <<status, lockedBy>>
    /\ UNCHANGED <<attempts, leaseLive, held, writes>>

Release(s) == Complete(s) \/ (\E r \in BOOLEAN : Fail(s, r)) \/ Cancel(s)

Next ==
    \/ \E s \in Slots : Claim(s) \/ Release(s)
    \/ Sweep \/ Expire \/ RequestCancel

\* Weak fairness: some idle slot eventually claims a claimable job, and a slot
\* that holds an attempt eventually finishes it. Expiry and cancellation are
\* not fair; they may never happen.
Fairness ==
    /\ WF_vars(\E s \in Slots : Claim(s))
    /\ \A s \in Slots : WF_vars(Release(s))

Spec == Init /\ [][Next]_vars
FairSpec == Spec /\ Fairness

(* Safety *)

\* Every owned write (success, failure, retry, cancel) came from the attempt
\* that holds the lease. This is "succeeded implies completed by the latest
\* attempt", applied to every terminal write.
WriteByLatestAttempt == \A w \in writes : w.attempt = w.latest

\* At most one terminal write per lease (attempt number).
OneTerminalWritePerLease ==
    \A w1, w2 \in writes :
        (w1.attempt = w2.attempt /\ w1.kind # "retry" /\ w2.kind # "retry") => w1 = w2

AttemptsBounded == attempts <= MaxAttempts

(* Liveness: a pending job is eventually claimed or cancelled. *)
PendingEventuallyClaimed == (status = "pending") ~> (status # "pending")

=============================================================================
