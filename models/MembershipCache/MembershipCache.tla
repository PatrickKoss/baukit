--------------------------- MODULE MembershipCache ---------------------------
(* Organization membership cached per replica, a hub that removes members or
   changes their role, and optional event delivery. See README.md for the
   mapping to baukit-suite plan_organization.rs. *)
EXTENDS Naturals

CONSTANTS
    Replicas,
    TTL,        \* ORGANIZATION_CACHE_TTL_SECONDS in ticks
    MaxTime,
    Design      \* "current", "fixed", or "naive" (fixed without the epoch check)

ASSUME Design \in {"current", "naive", "fixed"}

None == "none"
NoRead == [member |-> FALSE, role |-> None, epoch |-> 0, busy |-> FALSE]

VARIABLES
    now,
    member,      \* hub: the subject is a member of the organization
    role,        \* hub: "member" or "billing"
    removedAt,   \* hub: time of the removal (meaningful when ~member)
    until,       \* replica -> cached membership valid while now < until (0 = none)
    cRole,       \* replica -> role stored with the cache entry
    epoch,       \* replica -> evictions applied so far
    rd,          \* replica -> hub read in flight
    undelivered, \* replica -> a membership or role event not yet delivered
    delivered,   \* replica -> an event about the current hub state was delivered
    lateGrant,   \* granted after removal + TTL
    evictedGrant,\* granted after a removal or role event reached the replica
    removedGrant,\* granted after the removal event reached the replica
    billingGrant \* granted to a Billing-role member

vars == <<now, member, role, removedAt, until, cRole, epoch, rd, undelivered,
          delivered, lateGrant, evictedGrant, removedGrant, billingGrant>>

Init ==
    /\ now = 0
    /\ member = TRUE /\ role = "member" /\ removedAt = 0
    /\ until = [r \in Replicas |-> 0]
    /\ cRole = [r \in Replicas |-> None]
    /\ epoch = [r \in Replicas |-> 0]
    /\ rd = [r \in Replicas |-> NoRead]
    /\ undelivered = [r \in Replicas |-> FALSE]
    /\ delivered = [r \in Replicas |-> FALSE]
    /\ lateGrant = FALSE /\ evictedGrant = FALSE /\ removedGrant = FALSE
    /\ billingGrant = FALSE

Hub == <<member, role, removedAt>>
Flags == <<lateGrant, evictedGrant, removedGrant, billingGrant>>

\* Requests are much shorter than the TTL: time does not pass while a hub
\* read is in flight.
Tick ==
    /\ now < MaxTime
    /\ \A r \in Replicas : ~rd[r].busy
    /\ now' = now + 1
    /\ UNCHANGED <<member, role, removedAt, until, cRole, epoch, rd, undelivered,
                   delivered, Flags>>

\* organization.membership_changed / role change at the hub; the event goes to
\* every replica.
Emit ==
    /\ undelivered' = [r \in Replicas |-> TRUE]
    /\ delivered' = [r \in Replicas |-> FALSE]

Remove ==
    /\ member
    /\ member' = FALSE /\ removedAt' = now
    /\ Emit
    /\ UNCHANGED <<now, role, until, cRole, epoch, rd, Flags>>

ChangeRole ==
    /\ member /\ role = "member"
    /\ role' = "billing"
    /\ Emit
    /\ UNCHANGED <<now, member, removedAt, until, cRole, epoch, rd, Flags>>

\* Event delivery. The epoch counts events the replica received. Current: no
\* public evict exists, so the product cannot act on the event. Fixed:
\* evict(org, subject) drops the entry.
DeliverEvent(r) ==
    /\ undelivered[r]
    /\ undelivered' = [undelivered EXCEPT ![r] = FALSE]
    /\ delivered' = [delivered EXCEPT ![r] = TRUE]
    /\ epoch' = [epoch EXCEPT ![r] = @ + 1]
    /\ until' = IF Design # "current" THEN [until EXCEPT ![r] = 0] ELSE until
    /\ UNCHANGED <<now, member, role, removedAt, cRole, rd, Flags>>

\* afterEvent: the decision was made after the event reached the replica (a
\* hub read that started before delivery may still answer with the old state).
Grant(r, gRole, afterEvent) ==
    /\ lateGrant' = (lateGrant \/ (~member /\ now > removedAt + TTL))
    /\ evictedGrant' = (evictedGrant \/
                        (afterEvent /\ delivered[r] /\ (~member \/ role = "billing")))
    /\ removedGrant' = (removedGrant \/ (afterEvent /\ delivered[r] /\ ~member))
    /\ billingGrant' = (billingGrant \/ gRole = "billing")

\* Fixed: require_role(Member) checks the role is_member returns.
Allows(gRole) == Design = "current" \/ gRole = "member"

\* A request on replica r with a fresh cache entry: is_member returns true.
CachedRequest(r) ==
    /\ now < until[r]
    /\ IF Allows(cRole[r]) THEN Grant(r, cRole[r], TRUE) ELSE UNCHANGED Flags
    /\ UNCHANGED <<now, member, role, removedAt, until, cRole, epoch, rd,
                   undelivered, delivered>>

\* Cache miss: hub_member reads the hub (an await).
StartRead(r) ==
    /\ now >= until[r]
    /\ ~rd[r].busy
    /\ rd' = [rd EXCEPT ![r] = [member |-> member, role |-> role,
                                 epoch |-> epoch[r], busy |-> TRUE]]
    /\ UNCHANGED <<now, member, role, removedAt, until, cRole, epoch,
                   undelivered, delivered, Flags>>

\* The read returns: a member is cached for TTL and granted. Fixed: the entry
\* is stored only if no eviction ran since the read started. Without that
\* check ("naive"), a read that raced an eviction re-caches the removed
\* member for a full TTL.
FinishRead(r) ==
    /\ rd[r].busy
    /\ LET x == rd[r]
           store == x.member /\ (Design # "fixed" \/ x.epoch = epoch[r])
       IN /\ until' = IF store THEN [until EXCEPT ![r] = now + TTL] ELSE until
          /\ cRole' = IF store THEN [cRole EXCEPT ![r] = x.role] ELSE cRole
          /\ IF x.member /\ Allows(x.role) THEN Grant(r, x.role, x.epoch = epoch[r]) ELSE UNCHANGED Flags
    /\ rd' = [rd EXCEPT ![r] = NoRead]
    /\ UNCHANGED <<now, member, role, removedAt, epoch, undelivered, delivered>>

Next ==
    \/ Tick \/ Remove \/ ChangeRole
    \/ \E r \in Replicas :
        DeliverEvent(r) \/ CachedRequest(r) \/ StartRead(r) \/ FinishRead(r)

Spec == Init /\ [][Next]_vars

(* Invariants *)

\* Removed(s, o) and now - t_remove > TTL implies not granted.
NoGrantAfterTTL == ~lateGrant

\* With event-driven eviction, removal (or a role change away from Member)
\* implies not granted once the event reached the replica.
NoGrantAfterEvent == ~evictedGrant

\* The same, for removal alone (the F-39 eviction gap without the role gap).
NoGrantAfterRemovalEvent == ~removedGrant

\* A Billing-role member never passes require_role(Member).
BillingNeverGranted == ~billingGrant

=============================================================================
