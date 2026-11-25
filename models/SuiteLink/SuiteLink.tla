------------------------------ MODULE SuiteLink ------------------------------
(* Suite account linking between an initiator app I and an authorizer app A
   for one link request. See README.md for the mapping to baukit-suite. *)
EXTENDS Naturals, FiniteSets

CONSTANTS
    Completers,    \* concurrent complete() calls for the same request
    MaxExchanges   \* bound on exchange calls (each complete may retry once on timeout)

None == "none"
LinkI == "Li"      \* the initiator link id fixed by prepare_exchange
NoLink == [remote |-> None, status |-> None]

VARIABLES
    reqState,      \* I: "absent", "pending", "consumed", "expired"
    prepared,      \* I: prepared exchange stored on the request
    reqLink,       \* I: link id recorded on the consumed request
    iLinks,        \* I: inserted initiator links: set of [id, status]
    iInserts,      \* I: number of initiator link inserts for this request
    codeState,     \* A: "absent", "issued", "consumed", "expired"
    aLink,         \* A: authorizer link created for the code, NoLink before
    aInserts,      \* A: number of authorizer link inserts for this code
    exchanges,     \* exchange calls made so far
    pc,            \* completer -> "idle", "prepared", "exchanged", "done"
    result,        \* completer -> link id returned, or None
    outbox         \* pending peer revokes: subset of {"toA", "toI"}

vars == <<reqState, prepared, reqLink, iLinks, iInserts, codeState, aLink,
          aInserts, exchanges, pc, result, outbox>>

Init ==
    /\ reqState = "absent" /\ prepared = FALSE /\ reqLink = None
    /\ iLinks = {} /\ iInserts = 0
    /\ codeState = "absent" /\ aLink = NoLink /\ aInserts = 0
    /\ exchanges = 0
    /\ pc = [c \in Completers |-> "idle"]
    /\ result = [c \in Completers |-> None]
    /\ outbox = {}

Start ==
    /\ reqState = "absent"
    /\ reqState' = "pending"
    /\ UNCHANGED <<prepared, reqLink, iLinks, iInserts, codeState, aLink,
                   aInserts, exchanges, pc, result, outbox>>

\* authorize at A after the user consents; the callback carries the code to I.
Authorize ==
    /\ reqState = "pending" /\ codeState = "absent"
    /\ codeState' = "issued"
    /\ UNCHANGED <<reqState, prepared, reqLink, iLinks, iInserts, aLink,
                   aInserts, exchanges, pc, result, outbox>>

Expire ==
    \/ /\ reqState = "pending" /\ reqState' = "expired"
       /\ UNCHANGED codeState
    \/ /\ codeState = "issued" /\ codeState' = "expired"
       /\ UNCHANGED reqState
    /\ UNCHANGED <<prepared, reqLink, iLinks, iInserts, aLink, aInserts,
                   exchanges, pc, result, outbox>>

Finish(c, r) ==
    /\ pc' = [pc EXCEPT ![c] = "done"]
    /\ result' = [result EXCEPT ![c] = r]

LinkStatus(id) == IF \E l \in iLinks : l.id = id
                  THEN (CHOOSE l \in iLinks : l.id = id).status ELSE None

\* complete, first transaction: request FOR UPDATE, validate_complete,
\* prepare_exchange (keeps an existing prepared exchange), commit.
CompletePrepare(c) ==
    /\ pc[c] = "idle"
    /\ codeState # "absent"           \* the callback delivered a code
    /\ CASE reqState = "consumed" ->
              /\ Finish(c, IF LinkStatus(reqLink) = "revoked" THEN None ELSE reqLink)
              /\ UNCHANGED prepared
         [] reqState = "pending" ->
              /\ prepared' = TRUE
              /\ pc' = [pc EXCEPT ![c] = "prepared"]
              /\ UNCHANGED result
         [] OTHER -> Finish(c, None) /\ UNCHANGED prepared
    /\ UNCHANGED <<reqState, reqLink, iLinks, iInserts, codeState, aLink,
                   aInserts, exchanges, outbox>>

\* A's exchange_inner, one transaction: code FOR UPDATE, link for code,
\* validate_exchange, replace, consume_code. Seen from I, a call may also time
\* out after A committed; exchange_with_peer then calls once more.
ExchangeAtA(c, delivered) ==
    /\ pc[c] = "prepared"
    /\ exchanges < MaxExchanges
    /\ exchanges' = exchanges + 1
    /\ CASE codeState = "consumed" /\ aLink.status # None /\ aLink.remote = LinkI ->
              \* ExistingLink: idempotent answer
              UNCHANGED <<codeState, aLink, aInserts>>
         [] codeState = "issued" ->
              /\ codeState' = "consumed"
              /\ aLink' = [remote |-> LinkI, status |-> "active"]
              /\ aInserts' = aInserts + 1
         [] OTHER -> UNCHANGED <<codeState, aLink, aInserts>>
    /\ LET ok == codeState = "issued" \/ (codeState = "consumed" /\ aLink.status # None)
       IN IF ok /\ delivered
          THEN pc' = [pc EXCEPT ![c] = "exchanged"] /\ UNCHANGED result
          ELSE IF ok /\ ~delivered
          THEN UNCHANGED <<pc, result>>      \* timeout: retry from "prepared"
          ELSE Finish(c, None)
    /\ UNCHANGED <<reqState, prepared, reqLink, iLinks, iInserts, outbox>>

\* persist_completed, second transaction: active link FOR UPDATE, request FOR
\* UPDATE, validate_complete again, consume, replace, record link, commit.
Persist(c) ==
    /\ pc[c] = "exchanged"
    /\ CASE reqState = "consumed" ->
              /\ Finish(c, IF LinkStatus(reqLink) = "revoked" THEN None ELSE reqLink)
              /\ UNCHANGED <<reqState, reqLink, iLinks, iInserts>>
         [] reqState = "pending" ->
              /\ reqState' = "consumed"
              /\ reqLink' = LinkI
              /\ iLinks' = {[l EXCEPT !.status = "revoked"] : l \in iLinks}
                           \cup {[id |-> LinkI, status |-> "active"]}
              /\ iInserts' = iInserts + 1
              /\ Finish(c, LinkI)
         [] OTHER -> Finish(c, None) /\ UNCHANGED <<reqState, reqLink, iLinks, iInserts>>
    /\ UNCHANGED <<prepared, codeState, aLink, aInserts, exchanges, outbox>>

\* disconnect on either side: revoke locally and enqueue the peer revoke.
RevokeAtI ==
    /\ LinkStatus(LinkI) = "active"
    /\ iLinks' = {[l EXCEPT !.status = "revoked"] : l \in iLinks}
    /\ outbox' = outbox \cup {"toA"}
    /\ UNCHANGED <<reqState, prepared, reqLink, iInserts, codeState, aLink,
                   aInserts, exchanges, pc, result>>

RevokeAtA ==
    /\ aLink.status # None /\ aLink.status = "active"
    /\ aLink' = [aLink EXCEPT !.status = "revoked"]
    /\ outbox' = outbox \cup {"toI"}
    /\ UNCHANGED <<reqState, prepared, reqLink, iLinks, iInserts, codeState,
                   aInserts, exchanges, pc, result>>

DeliverRevoke ==
    \/ /\ "toA" \in outbox
       /\ aLink' = IF aLink = NoLink THEN NoLink ELSE [aLink EXCEPT !.status = "revoked"]
       /\ outbox' = outbox \ {"toA"}
       /\ UNCHANGED iLinks
    \/ /\ "toI" \in outbox
       /\ iLinks' = {[l EXCEPT !.status = "revoked"] : l \in iLinks}
       /\ outbox' = outbox \ {"toI"}
       /\ UNCHANGED aLink
    /\ UNCHANGED <<reqState, prepared, reqLink, iInserts, codeState, aInserts,
                   exchanges, pc, result>>

Next ==
    \/ Start \/ Authorize \/ Expire
    \/ \E c \in Completers : CompletePrepare(c) \/ Persist(c)
                             \/ \E d \in BOOLEAN : ExchangeAtA(c, d)
    \/ RevokeAtI \/ RevokeAtA \/ DeliverRevoke

Spec == Init /\ [][Next]_vars

(* Invariants *)

AtMostOneLinkPerRequest == iInserts <= 1 /\ aInserts <= 1

ConsumedImpliesLinkId == reqState = "consumed" => reqLink # None

\* Every complete that returns a link returns the same one.
CompletionIdempotent ==
    \A c1, c2 \in Completers :
        (result[c1] # None /\ result[c2] # None) => result[c1] = result[c2]

\* An active initiator link implies the authorizer created its side.
InitiatorImpliesAuthorizer == LinkStatus(LinkI) = "active" => aLink.status # None

=============================================================================
