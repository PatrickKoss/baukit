----------------------------- MODULE SuiteInbox -----------------------------
(* Hub-to-product platform event inbox: at-least-once redelivery, prepare
   outside the transaction, cancellation, receipt purge. See README.md for the
   mapping to baukit-suite platform/postgres.rs. *)
EXTENDS Naturals, FiniteSets

CONSTANTS
    Events,      \* event ids; the event's revision is its position in RevOf
    Workers,     \* concurrent HTTP requests the hub may have in flight
    MaxEffects,  \* state bound so a violating run stays finite
    MaxCancels   \* cancellations are finite: the liveness assumption

\* Two events for one subject, revisions 1 and 2.
RevOf == [e \in Events |-> IF e = "e1" THEN 1 ELSE 2]
Bodies == {"good", "tampered"}
None == "none"

VARIABLES
    receipt,     \* event -> stored body hash, or None (never stored or purged)
    stateRev,    \* product state: highest revision applied by the handler
    effects,     \* event -> times the handler changed product state for it
    acked,       \* event -> the hub saw a 2xx for the good body
    pc,          \* worker -> "idle", "prepared", "done"
    job,         \* worker -> [ev, body]
    cancels,     \* cancellations so far
    conflictOk   \* every changed-body delivery against a stored receipt got 409

vars == <<receipt, stateRev, effects, acked, pc, job, conflictOk, cancels>>

Init ==
    /\ receipt = [e \in Events |-> None]
    /\ stateRev = 0
    /\ effects = [e \in Events |-> 0]
    /\ acked = [e \in Events |-> FALSE]
    /\ pc = [w \in Workers |-> "idle"]
    /\ job = [w \in Workers |-> [ev |-> "e1", body |-> "good"]]
    /\ conflictOk = TRUE
    /\ cancels = 0

\* The hub (re)sends an event. Good bodies are resent until acked and may be
\* resent after an ack (lost response). A changed body under the same id is
\* the attacker or a hub bug.
Send(w, e, b) ==
    /\ pc[w] = "idle"
    /\ effects[e] < MaxEffects
    \* Bodies are HMAC-signed, so only the hub can change one; it does so only
    \* for an id it already delivered.
    /\ b = "tampered" => acked[e]
    /\ job' = [job EXCEPT ![w] = [ev |-> e, body |-> b]]
    /\ pc' = [pc EXCEPT ![w] = "sent"]
    /\ UNCHANGED <<receipt, stateRev, effects, acked, conflictOk>>

Respond(w, ok) ==
    /\ pc' = [pc EXCEPT ![w] = "idle"]
    /\ acked' = IF ok /\ job[w].body = "good"
                THEN [acked EXCEPT ![job[w].ev] = TRUE] ELSE acked

\* receive, step 1: read the receipt outside any transaction. A stored receipt
\* answers duplicate (same hash) or 409 (changed hash). Otherwise prepare runs
\* outside the transaction.
PreRead(w) ==
    /\ pc[w] = "sent"
    /\ LET r == receipt[job[w].ev] IN
       IF r = None
       THEN /\ pc' = [pc EXCEPT ![w] = "prepared"]
            /\ UNCHANGED <<acked, conflictOk>>
       ELSE /\ Respond(w, r = job[w].body)
            /\ conflictOk' = (conflictOk /\ (r # job[w].body => pc'[w] = "idle"))
    /\ UNCHANGED <<receipt, stateRev, effects, job>>

\* receive, step 2: one transaction inserts the receipt ON CONFLICT DO NOTHING
\* and runs the revision-guarded handler, then commits.
Commit(w) ==
    /\ pc[w] = "prepared"
    /\ LET e == job[w].ev
           r == receipt[e]
       IN IF r # None
          THEN /\ Respond(w, r = job[w].body)
               /\ UNCHANGED <<receipt, stateRev, effects, conflictOk, job>>
               \* the in-transaction re-read gives the same duplicate or 409
          ELSE /\ receipt' = [receipt EXCEPT ![e] = job[w].body]
               /\ IF RevOf[e] > stateRev
                  THEN /\ stateRev' = RevOf[e]
                       /\ effects' = [effects EXCEPT ![e] = @ + 1]
                  ELSE UNCHANGED <<stateRev, effects>>
               /\ Respond(w, TRUE)
               /\ UNCHANGED conflictOk
    /\ UNCHANGED job

\* The request future is dropped (client timeout, shutdown) before commit, or
\* prepare fails: nothing was written; the hub retries.
Cancel(w) ==
    /\ pc[w] \in {"sent", "prepared"}
    /\ cancels < MaxCancels
    /\ cancels' = cancels + 1
    /\ Respond(w, FALSE)
    /\ UNCHANGED <<receipt, stateRev, effects, conflictOk, job>>

\* purge_received_before after 30 days: any stored receipt may go.
Purge(e) ==
    /\ receipt[e] # None
    /\ receipt' = [receipt EXCEPT ![e] = None]
    /\ UNCHANGED <<stateRev, effects, acked, pc, job, conflictOk>>

Next ==
    \/ \E w \in Workers :
        \/ \E e \in Events, b \in Bodies : Send(w, e, b) /\ UNCHANGED cancels
        \/ PreRead(w) /\ UNCHANGED cancels
        \/ Commit(w) /\ UNCHANGED cancels
        \/ Cancel(w)
    \/ \E e \in Events : Purge(e) /\ UNCHANGED cancels

\* The hub keeps sending each unacked good event: strong fairness, because a
\* free worker slot comes and goes while other events are in flight. Once
\* sent, a request is processed (weak fairness). Purge is unfair.
\* Cancellation happens at most MaxCancels times, which stands in for "not
\* forever".
Fairness ==
    /\ \A e \in Events :
        SF_vars(\E w \in Workers : ~acked[e] /\ Send(w, e, "good") /\ UNCHANGED cancels)
    /\ \A w \in Workers :
        /\ WF_vars(PreRead(w) /\ UNCHANGED cancels)
        /\ WF_vars(Commit(w) /\ UNCHANGED cancels)

Spec == Init /\ [][Next]_vars
FairSpec == Spec /\ Fairness

(* Invariants *)

\* The revision-guarded handler changes product state at most once per event,
\* also after its receipt was purged and the event redelivered.
ExactlyOneEffect == \A e \in Events : effects[e] <= 1

\* An ack for an event means its revision (or a newer one) is applied.
AckedImpliesApplied == \A e \in Events : acked[e] => stateRev >= RevOf[e]

\* A changed body for a stored id is never acknowledged as applied.
ChangedBodyIs409 == conflictOk

(* Liveness: at-least-once delivery. *)
EveryEventAcked == \A e \in Events : <>acked[e]

=============================================================================
