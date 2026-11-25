-------------------------- MODULE BffSessionFamily --------------------------
(* One BFF session family, the IdP's refresh-token family with reuse
   detection, and an honest browser with several tabs sharing one cookie jar.
   See README.md for the mapping to baukit-auth browser_session. *)
EXTENDS Naturals, FiniteSets

CONSTANTS
    Tabs,
    MaxRows,        \* bound on session rows (rotations + 1)
    Design,         \* "current" or "fixed"
    ResponseLoss,   \* the browser may lose a response after the store committed
    GraceCanLapse   \* the 10 s replay grace may pass before the browser retries

ASSUME Design \in {"current", "fixed"}
ASSUME ResponseLoss \in BOOLEAN /\ GraceCanLapse \in BOOLEAN

Rows == 1..MaxRows
None == "none"

VARIABLES
    idpCur,       \* the IdP's only valid refresh token (tokens are 1, 2, ...)
    idpRevoked,   \* the IdP revoked the token family
    nrows,        \* rows 1..nrows exist; row k rotated to k + 1 when k < nrows
    rowTok,       \* refresh token sealed in each row
    graceOpen,    \* row k still serves its grace access token to a replay
    famRevoked,   \* revoked_at set on the family (the user is logged out)
    lock,         \* tab holding the row lock (FOR UPDATE), or None
    cookie,       \* row the cookie jar points at
    pc,           \* tab -> "idle", "locked", "rotated", "detached"
    newTok,       \* tab -> token the IdP returned to this request
    loggedOut,    \* set when the family is revoked
    accessAfterRevoke

vars == <<idpCur, idpRevoked, nrows, rowTok, graceOpen, famRevoked, lock,
          cookie, pc, newTok, loggedOut, accessAfterRevoke>>

Init ==
    /\ idpCur = 1
    /\ idpRevoked = FALSE
    /\ nrows = 1
    /\ rowTok = [k \in Rows |-> IF k = 1 THEN 1 ELSE 0]
    /\ graceOpen = [k \in Rows |-> FALSE]
    /\ famRevoked = FALSE
    /\ lock = None
    /\ cookie = 1
    /\ pc = [t \in Tabs |-> "idle"]
    /\ newTok = [t \in Tabs |-> 0]
    /\ loggedOut = FALSE
    /\ accessAfterRevoke = FALSE

Rotated(k) == k < nrows

RevokeFamily ==
    /\ famRevoked' = TRUE
    /\ idpRevoked' = TRUE        \* revoke_at_idp on the head
    /\ loggedOut' = TRUE

\* refresh: begin, locked_match (FOR UPDATE on the cookie's row).
Begin(t) ==
    /\ pc[t] = "idle"
    /\ lock = None
    /\ ~famRevoked
    /\ lock' = t
    /\ pc' = [pc EXCEPT ![t] = "locked"]
    /\ UNCHANGED <<idpCur, idpRevoked, nrows, rowTok, graceOpen, famRevoked,
                   cookie, newTok, loggedOut, accessAfterRevoke>>

Unlock(t) == lock' = None /\ pc' = [pc EXCEPT ![t] = "idle"]

\* row.rotated_to is set: replay. Within grace the successor's access token is
\* served with no cookie; after grace the family is revoked.
Replay(t) ==
    /\ pc[t] = "locked"
    /\ Rotated(cookie)
    /\ IF graceOpen[cookie + 1]
       THEN /\ accessAfterRevoke' = (accessAfterRevoke \/ famRevoked)
            /\ UNCHANGED <<famRevoked, idpRevoked, loggedOut>>
       ELSE RevokeFamily /\ UNCHANGED accessAfterRevoke
    /\ Unlock(t)
    /\ UNCHANGED <<idpCur, nrows, rowTok, graceOpen, cookie, newTok>>

\* idp.refresh(stored token) while the transaction stays open.
IdpRefresh(t) ==
    /\ pc[t] = "locked"
    /\ ~Rotated(cookie)
    /\ nrows < MaxRows
    /\ LET tok == rowTok[cookie] IN
       IF idpRevoked \/ tok # idpCur
       THEN \* InvalidGrant. A stale token also trips reuse detection.
            /\ RevokeFamily
            /\ Unlock(t)
            /\ UNCHANGED <<idpCur, newTok>>
       ELSE /\ idpCur' = idpCur + 1
            /\ newTok' = [newTok EXCEPT ![t] = idpCur + 1]
            /\ pc' = [pc EXCEPT ![t] = IF Design = "fixed" THEN "detached" ELSE "rotated"]
            /\ UNCHANGED <<idpRevoked, famRevoked, loggedOut, lock>>
    /\ UNCHANGED <<nrows, rowTok, graceOpen, cookie, accessAfterRevoke>>

\* rotate: insert successor, mark_rotated, commit. The browser gets the new
\* cookie unless the response is lost.
CommitStore(t, delivered) ==
    /\ pc[t] \in {"rotated", "detached"}
    /\ delivered \/ ResponseLoss
    /\ nrows' = nrows + 1
    /\ rowTok' = [rowTok EXCEPT ![nrows + 1] = newTok[t]]
    /\ graceOpen' = [graceOpen EXCEPT ![nrows + 1] = TRUE]
    /\ cookie' = IF delivered THEN nrows + 1 ELSE cookie
    /\ accessAfterRevoke' = (accessAfterRevoke \/ (delivered /\ famRevoked))
    /\ Unlock(t)
    /\ UNCHANGED <<idpCur, idpRevoked, famRevoked, newTok, loggedOut>>

\* Current design only: the client disconnects or the commit fails after the
\* IdP rotated. The future is dropped and the transaction rolls back. In the
\* fixed design the store runs in a detached task and retries the commit with
\* the new token, so this step does not exist.
CancelAfterRotate(t) ==
    /\ pc[t] = "rotated"
    /\ Unlock(t)
    /\ UNCHANGED <<idpCur, idpRevoked, nrows, rowTok, graceOpen, famRevoked,
                   cookie, newTok, loggedOut, accessAfterRevoke>>

GraceLapse(k) ==
    /\ GraceCanLapse
    /\ graceOpen[k]
    /\ graceOpen' = [graceOpen EXCEPT ![k] = FALSE]
    /\ UNCHANGED <<idpCur, idpRevoked, nrows, rowTok, famRevoked, lock, cookie,
                   pc, newTok, loggedOut, accessAfterRevoke>>

Next ==
    \/ \E t \in Tabs :
        \/ Begin(t) \/ Replay(t) \/ IdpRefresh(t) \/ CancelAfterRotate(t)
        \/ \E d \in BOOLEAN : CommitStore(t, d)
    \/ \E k \in Rows : GraceLapse(k)

Spec == Init /\ [][Next]_vars

(* Invariants *)

\* At most one live (unrotated, unrevoked) head per family.
OneLiveHead == Cardinality({k \in 1..nrows : ~Rotated(k) /\ ~famRevoked}) <= 1

\* No access token is handed out for a revoked family.
NoAccessForRevokedFamily == ~accessAfterRevoke

\* When no refresh is in flight, the stored head token is the IdP's current
\* token, so the next refresh never presents a rotated token to the IdP.
StoreTracksIdp ==
    (lock = None /\ ~famRevoked) => rowTok[nrows] = idpCur

\* The browser is honest and only ever retries, so it is never logged out.
HonestNeverLoggedOut == ~loggedOut

=============================================================================
