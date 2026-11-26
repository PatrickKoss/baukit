------------------------- MODULE ClientAccountSwitch -------------------------
(* One device, several tabs sharing a session token, a lifecycle in the tab
   under test, per-subject partitions with an outbox, and server-side erasure.
   See README.md for the mapping to the TypeScript packages. *)
EXTENDS Naturals

CONSTANTS
    Subjects,
    MaxGen,      \* bound on session generations (sign-ins and sign-outs)
    Design       \* "current" or "fixed"

ASSUME Design \in {"current", "fixed"}

None == "none"
NoReq == [subj |-> None, gen |-> 0]

VARIABLES
    token,        \* the shared session: subject or None (other tabs replace it)
    gen,          \* session generation, bumped on every sign-in and sign-out
    active,       \* tab 1 lifecycle: subject of the ready partition, or None
    lastSel,      \* tab 1 lifecycle: lastSelectedPartition
    pending,      \* a session change broadcast tab 1 has not applied yet
    partition,    \* subject -> local partition exists
    outbox,       \* subject -> partition has unsynced changes
    serverErased, \* subject -> erased on the server
    erasedMark,   \* subject -> durable "erased" marker (fixed design only)
    erasingMem,   \* in-memory erased flag of erasure.ts (lost on reload), or None
    erasedByMe,   \* subject -> this device erased it on the server
    inflight,     \* tab 1 request in flight: [subj, gen] stamped at send, or NoReq
    adopt,        \* tab 1 adopt: NoReq, or [subj, gen] after the subject recheck
    badPush,      \* a push from partition p carried another subject's token
    badErase,     \* a profile_erased signal deleted a partition of a live subject
    pushAfterErase \* a push for s after this device erased s on the server

vars == <<token, gen, active, lastSel, pending, partition, outbox, serverErased,
          erasedMark, erasingMem, erasedByMe, inflight, adopt, badPush, badErase,
          pushAfterErase>>

Init ==
    /\ token = None /\ gen = 0
    /\ active = None /\ lastSel = None /\ pending = FALSE
    /\ partition = [s \in Subjects |-> FALSE]
    /\ outbox = [s \in Subjects |-> FALSE]
    /\ serverErased = [s \in Subjects |-> FALSE]
    /\ erasedMark = [s \in Subjects |-> FALSE]
    /\ erasingMem = None
    /\ erasedByMe = [s \in Subjects |-> FALSE]
    /\ inflight = NoReq /\ adopt = NoReq
    /\ badPush = FALSE /\ badErase = FALSE /\ pushAfterErase = FALSE

Flags == <<badPush, badErase, pushAfterErase>>
Erasure == <<erasingMem, erasedByMe>>

\* Sign-in or sign-out in any tab: the token changes now, the broadcast reaches
\* tab 1 later. clearSession in the template broadcasts the same way.
SignIn(s) ==
    /\ gen < MaxGen
    /\ token # s
    /\ ~serverErased[s]
    /\ token' = s /\ gen' = gen + 1 /\ pending' = TRUE
    /\ UNCHANGED <<active, lastSel, partition, outbox, serverErased, erasedMark,
                   Erasure, inflight, adopt, Flags>>

SignOut ==
    /\ gen < MaxGen
    /\ token # None
    /\ token' = None /\ gen' = gen + 1 /\ pending' = TRUE
    /\ UNCHANGED <<active, lastSel, partition, outbox, serverErased, erasedMark,
                   Erasure, inflight, adopt, Flags>>

\* Tab 1 applies the broadcast: selectSubject(token) or clear(). A selected
\* subject's partition is opened (created if missing). The fixed design
\* refuses to open a partition with a durable erased marker.
Deliver ==
    /\ pending
    /\ pending' = FALSE
    /\ IF token = None
       THEN active' = None /\ UNCHANGED <<lastSel, partition>>
       ELSE IF Design = "fixed" /\ erasedMark[token]
       THEN active' = None /\ UNCHANGED <<lastSel, partition>>
       ELSE /\ active' = token
            /\ lastSel' = token
            /\ partition' = [partition EXCEPT ![token] = TRUE]
    /\ UNCHANGED <<token, gen, outbox, serverErased, erasedMark, Erasure, inflight,
                   adopt, Flags>>

\* Local edit in the ready partition.
Edit ==
    /\ active # None
    /\ outbox' = [outbox EXCEPT ![active] = TRUE]
    /\ UNCHANGED <<token, gen, active, lastSel, pending, partition, serverErased,
                   erasedMark, Erasure, inflight, adopt, Flags>>

\* api-runtime sends a request with the token current at send time.
Request ==
    /\ inflight = NoReq /\ token # None
    /\ inflight' = [subj |-> token, gen |-> gen]
    /\ UNCHANGED <<token, gen, active, lastSel, pending, partition, outbox,
                   serverErased, erasedMark, Erasure, adopt, Flags>>

\* The server erases s (another device, or the data-subject API).
EraseServerElsewhere(s) ==
    /\ ~serverErased[s]
    /\ serverErased' = [serverErased EXCEPT ![s] = TRUE]
    /\ UNCHANGED <<token, gen, active, lastSel, pending, partition, outbox,
                   erasedMark, Erasure, inflight, adopt, Flags>>

LocalErase(s) ==
    /\ partition' = [partition EXCEPT ![s] = FALSE]
    /\ outbox' = [outbox EXCEPT ![s] = FALSE]
    /\ badErase' = (badErase \/ ~serverErased[s])

\* The response arrives. For an erased subject it is 401 profile_erased and
\* api-runtime fires onProfileErased; the template calls handleProfileErased
\* and clearSession.
\* Current: no subject, no session check; erases lastSelectedPartition.
\* Fixed: dropped if the request's generation is stale; erases only its subject.
Respond ==
    /\ inflight # NoReq
    /\ inflight' = NoReq
    /\ LET s == inflight.subj
           stale == inflight.gen # gen
       IN IF ~serverErased[s] \/ (Design = "fixed" /\ stale)
          THEN UNCHANGED <<token, gen, active, pending, partition, outbox, badErase>>
          ELSE LET victim == IF Design = "fixed" THEN s ELSE lastSel IN
               /\ IF victim = None
                  THEN UNCHANGED <<partition, outbox, badErase>>
                  ELSE LocalErase(victim)
               /\ active' = None
               /\ token' = None
               /\ gen' = IF gen < MaxGen THEN gen + 1 ELSE gen
               /\ pending' = FALSE
    /\ UNCHANGED <<lastSel, serverErased, erasedMark, Erasure, adopt, badPush,
                   pushAfterErase>>

\* recheckServerSubjectBeforeSyncAdoption: the server confirms the token's
\* subject equals the active partition's subject.
Recheck ==
    /\ adopt = NoReq
    /\ erasingMem = None
    /\ active # None /\ outbox[active]
    /\ token = active
    /\ adopt' = [subj |-> active, gen |-> gen]
    /\ UNCHANGED <<token, gen, active, lastSel, pending, partition, outbox,
                   serverErased, erasedMark, Erasure, inflight, Flags>>

\* adopt() pushes the partition's outbox, reading the token at send time.
\* Fixed: the token provider refuses a token from another generation, so a
\* switch between recheck and push aborts the adopt.
Push ==
    /\ adopt # NoReq
    /\ adopt' = NoReq
    /\ LET p == adopt.subj IN
       IF token = None \/ (Design = "fixed" /\ adopt.gen # gen) \/ ~partition[p]
       THEN UNCHANGED <<outbox, badPush, pushAfterErase>>
       ELSE /\ outbox' = [outbox EXCEPT ![p] = FALSE]
            /\ badPush' = (badPush \/ token # p)
            /\ pushAfterErase' = (pushAfterErase \/ erasedByMe[p])
    /\ UNCHANGED <<token, gen, active, lastSel, pending, partition, serverErased,
                   erasedMark, Erasure, inflight, badErase>>

\* This device erases its own profile (erasure.ts): stop sync, mark erased,
\* server erase, then delete the partition and sign out. Current: the erased
\* flag lives in memory. Fixed: a durable marker is written before the server
\* call and cleared after the local delete.
EraseStart ==
    /\ active # None /\ erasingMem = None
    /\ erasingMem' = active
    /\ adopt' = NoReq
    /\ erasedMark' = IF Design = "fixed" THEN [erasedMark EXCEPT ![active] = TRUE]
                     ELSE erasedMark
    /\ UNCHANGED <<token, gen, active, lastSel, pending, partition, outbox,
                   serverErased, erasedByMe, inflight, Flags>>

EraseServerCall ==
    /\ erasingMem # None
    /\ ~erasedByMe[erasingMem]
    /\ serverErased' = [serverErased EXCEPT ![erasingMem] = TRUE]
    /\ erasedByMe' = [erasedByMe EXCEPT ![erasingMem] = TRUE]
    /\ UNCHANGED <<token, gen, active, lastSel, pending, partition, outbox,
                   erasedMark, erasingMem, inflight, adopt, Flags>>

EraseLocalDelete ==
    /\ erasingMem # None /\ erasedByMe[erasingMem]
    /\ LocalErase(erasingMem)
    /\ erasedMark' = [erasedMark EXCEPT ![erasingMem] = FALSE]
    /\ erasingMem' = None
    /\ active' = None
    /\ token' = None /\ gen' = IF gen < MaxGen THEN gen + 1 ELSE gen
    /\ pending' = FALSE
    /\ UNCHANGED <<lastSel, serverErased, erasedByMe, inflight, adopt, badPush,
                   pushAfterErase>>

\* Reload or crash of tab 1: memory is gone, durable state stays. The tab
\* reads the shared session again on start.
Reload ==
    /\ active' = None /\ lastSel' = None
    /\ erasingMem' = None /\ adopt' = NoReq /\ inflight' = NoReq
    /\ pending' = (token # None)
    /\ UNCHANGED <<token, gen, partition, outbox, serverErased, erasedMark,
                   erasedByMe, Flags>>

Next ==
    \/ \E s \in Subjects : SignIn(s) \/ EraseServerElsewhere(s)
    \/ SignOut \/ Deliver \/ Edit \/ Request \/ Respond \/ Recheck \/ Push
    \/ EraseStart \/ EraseServerCall \/ EraseLocalDelete \/ Reload

Spec == Init /\ [][Next]_vars

(* Invariants *)

\* Every push from partition p carries subject(p)'s token (F-53).
PushCarriesOwnToken == ~badPush

\* A profile_erased signal only deletes partitions of erased subjects (F-38).
EraseTouchesOnlyErased == ~badErase

\* No push for s after this device erased s on the server (F-54).
NoPushAfterErase == ~pushAfterErase

=============================================================================
