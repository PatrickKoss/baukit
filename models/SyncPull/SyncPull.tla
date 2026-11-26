------------------------------ MODULE SyncPull ------------------------------
(* Paged sync pull against concurrent writers and tombstone purge.
   See README.md for the mapping to baukit-sync. *)
EXTENDS Naturals, FiniteSets

CONSTANTS
    Rows,        \* row ids 1..n, ordered as naturals (the id tie-breaker)
    MaxRev,      \* bound on the owner's revision counter
    Design,      \* "current" or "fixed"
    Isolation,   \* "RC" or "RR"
    SameRevWrites, \* whether a push may stamp one revision on two rows
    Paged        \* TRUE: LIMIT 1 per table query; FALSE: each query returns every eligible row

ASSUME SameRevWrites \in BOOLEAN /\ Paged \in BOOLEAN
ASSUME Design \in {"current", "fixed"} /\ Isolation \in {"RC", "RR"}

\* Two tables: the highest row id lives in table 2, the others in table 1.
Tables == {1, 2}
TableOf == [r \in Rows |-> IF \A o \in Rows : o <= r THEN 2 ELSE 1]

Absent == [rev |-> 0, live |-> FALSE]
MaxId == CHOOSE m \in Rows : \A r \in Rows : r <= m
Max(S) == CHOOSE m \in S : \A x \in S : x <= m

\* Lexicographic order on (revision, id) keys.
KeyLt(a, b) == a[1] < b[1] \/ (a[1] = b[1] /\ a[2] < b[2])
KeyLe(a, b) == a = b \/ KeyLt(a, b)

VARIABLES
    counter,   \* sync_revisions.last_revision (committed)
    srv,       \* committed server rows: row -> [rev, live]
    horizon,   \* sync_purge_horizons.horizon_revision
    cursor,    \* client cursor as a (revision, id) key
    replica,   \* client copy: row -> [rev, live]
    pc,        \* puller: "idle", "reading", "done"
    snap,      \* RR snapshot of srv taken at the first statement
    hi,        \* fixed design: high-water mark read first
    toRead,    \* tables not yet queried in this pull
    page       \* rows returned so far in this pull: set of [row, ver]

vars == <<counter, srv, horizon, cursor, replica, pc, snap, hi, toRead, page>>

Ver == [rev : 0..MaxRev, live : BOOLEAN]

TypeOK ==
    /\ counter \in 0..MaxRev
    /\ srv \in [Rows -> Ver]
    /\ replica \in [Rows -> Ver]
    /\ horizon \in 0..MaxRev
    /\ cursor \in (0..MaxRev) \X (0..MaxId)
    /\ pc \in {"idle", "reading"}

Init ==
    /\ counter = 0
    /\ srv = [r \in Rows |-> Absent]
    /\ horizon = 0
    /\ cursor = <<0, 0>>
    /\ replica = [r \in Rows |-> Absent]
    /\ pc = "idle"
    /\ snap = [r \in Rows |-> Absent]
    /\ hi = 0
    /\ toRead = {}
    /\ page = {}

Unchanged_puller == UNCHANGED <<cursor, replica, pc, snap, hi, toRead, page>>

(* Writers. Each write is one transaction that holds the owner's counter row
   (UPDATE sync_revisions ... RETURNING) until commit, so writers for one owner
   commit in revision order. Two writers are therefore modelled as one
   interleaving of atomic commits. *)

\* One row, one new revision (live write or tombstone).
WriteOne(r, live) ==
    /\ counter < MaxRev
    /\ counter' = counter + 1
    /\ srv' = [srv EXCEPT ![r] = [rev |-> counter + 1, live |-> live]]
    /\ UNCHANGED horizon /\ Unchanged_puller

\* The crate example: one revision stamped on every row of a push (F-06).
WriteSameRev(r1, r2) ==
    /\ SameRevWrites
    /\ r1 < r2
    /\ counter < MaxRev
    /\ counter' = counter + 1
    /\ srv' = [srv EXCEPT ![r1] = [rev |-> counter + 1, live |-> TRUE],
                          ![r2] = [rev |-> counter + 1, live |-> TRUE]]
    /\ UNCHANGED horizon /\ Unchanged_puller

\* One transaction writing two tables with two revisions (F-07).
WriteCross(r1, r2) ==
    /\ TableOf[r1] # TableOf[r2]
    /\ counter + 2 <= MaxRev
    /\ counter' = counter + 2
    /\ srv' = [srv EXCEPT ![r1] = [rev |-> counter + 1, live |-> TRUE],
                          ![r2] = [rev |-> counter + 2, live |-> TRUE]]
    /\ UNCHANGED horizon /\ Unchanged_puller

(* purge_tombstones: deletes a tombstone and raises the horizon in one batch
   transaction. It locks sync_revisions FOR UPDATE, which conflicts with the
   pull guard's FOR KEY SHARE, so it cannot commit while a pull is open. *)
Purge(r) ==
    /\ pc = "idle"
    /\ srv[r].rev > 0 /\ ~srv[r].live
    /\ srv' = [srv EXCEPT ![r] = Absent]
    /\ horizon' = IF srv[r].rev > horizon THEN srv[r].rev ELSE horizon
    /\ UNCHANGED counter /\ Unchanged_puller

(* Puller. guard_pull_cursor runs first in the pull transaction. A cursor
   below the horizon (and not 0) gets resync_required; the client restarts
   from 0 with an empty replica. *)
Begin ==
    /\ pc = "idle"
    /\ IF cursor[1] > 0 /\ cursor[1] < horizon
       THEN /\ cursor' = <<0, 0>>
            /\ replica' = [r \in Rows |-> Absent]
            /\ UNCHANGED <<pc, snap, hi, toRead, page>>
       ELSE /\ pc' = "reading"
            /\ snap' = srv
            /\ hi' = counter
            /\ toRead' = Tables
            /\ page' = {}
            /\ UNCHANGED <<cursor, replica>>
    /\ UNCHANGED <<counter, srv, horizon>>

View == IF Isolation = "RR" THEN snap ELSE srv

Eligible(v, t) ==
    {r \in Rows : /\ TableOf[r] = t
                  /\ v[r].rev > 0
                  /\ IF Design = "current"
                     THEN v[r].rev > cursor[1]
                     ELSE KeyLt(cursor, <<v[r].rev, r>>) /\ v[r].rev <= hi}

\* Rows that ORDER BY revision LIMIT 1 may return. The current query orders by
\* revision only, so ties come back in any order.
Firsts(v, t) ==
    LET E == Eligible(v, t) IN
    IF Design = "current"
    THEN {r \in E : \A o \in E : v[r].rev <= v[o].rev}
    ELSE {r \in E : \A o \in E : KeyLe(<<v[r].rev, r>>, <<v[o].rev, o>>)}

\* One table query (page size 1) as its own statement.
ReadTable(t) ==
    /\ pc = "reading"
    /\ t \in toRead
    /\ LET v == View IN
       IF ~Paged
       THEN page' = page \cup {[row |-> r, ver |-> v[r]] : r \in Eligible(v, t)}
       ELSE IF Firsts(v, t) = {}
       THEN page' = page
       ELSE \E r \in Firsts(v, t) : page' = page \cup {[row |-> r, ver |-> v[r]]}
    /\ toRead' = toRead \ {t}
    /\ UNCHANGED <<counter, srv, horizon, cursor, replica, pc, snap, hi>>

KeyOf(p) == <<p.ver.rev, p.row>>

\* Commit and apply. Current: the client stores the largest revision it saw
\* and applies every returned row. Fixed: apply only the smallest key across
\* tables (a merged page of 1), or jump to (hi, MaxId) when nothing is left.
Finish ==
    /\ pc = "reading"
    /\ toRead = {}
    /\ IF Design = "current"
       THEN /\ replica' = [r \in Rows |->
                    IF \E p \in page : p.row = r
                    THEN (CHOOSE p \in page : p.row = r).ver
                    ELSE replica[r]]
            /\ cursor' = IF page = {} THEN cursor
                         ELSE <<Max({cursor[1]} \cup {p.ver.rev : p \in page}), MaxId>>
       ELSE IF page = {}
            THEN /\ cursor' = IF KeyLt(cursor, <<hi, MaxId>>) THEN <<hi, MaxId>> ELSE cursor
                 /\ replica' = replica
            ELSE LET m == CHOOSE p \in page : \A o \in page : KeyLe(KeyOf(p), KeyOf(o))
                 IN /\ replica' = [replica EXCEPT ![m.row] = m.ver]
                    /\ cursor' = KeyOf(m)
    /\ pc' = "idle"
    /\ page' = {}
    /\ UNCHANGED <<counter, srv, horizon, snap, hi, toRead>>

Next ==
    \/ \E r \in Rows, l \in BOOLEAN : WriteOne(r, l)
    \/ \E r1, r2 \in Rows : WriteSameRev(r1, r2)
    \/ \E r1, r2 \in Rows : WriteCross(r1, r2)
    \/ \E r \in Rows : Purge(r)
    \/ Begin
    \/ \E t \in Tables : ReadTable(t)
    \/ Finish

Spec == Init /\ [][Next]_vars

(* Invariants, checked whenever no pull is open (the client's committed view). *)

\* Cursor c means every row version at or below c was applied.
NoLostUpdate ==
    pc = "idle" =>
        \A r \in Rows :
            (srv[r].rev > 0 /\ KeyLe(<<srv[r].rev, r>>, cursor)) => replica[r] = srv[r]

\* A purged row never stays live on a client whose cursor the guard accepts.
NoResurrection ==
    (pc = "idle" /\ ~(cursor[1] > 0 /\ cursor[1] < horizon)) =>
        \A r \in Rows : (srv[r] = Absent /\ replica[r].live) =>
            \* the row was never live on the server at a revision <= cursor
            FALSE

\* No purge raised the horizon above an open pull's start cursor.
HorizonSafety == pc = "reading" => (cursor[1] = 0 \/ cursor[1] >= horizon)

=============================================================================
