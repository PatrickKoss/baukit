# M-08 MembershipCache

Organization membership cached per replica, a hub that removes a member or changes their role, a clock,
and optional event delivery to each replica. Settles F-39.

## What maps to what

| Model | Code (`rust/crates/baukit-suite/src/platform/plan_organization.rs`) |
|---|---|
| `until`, `TTL` | `HubOrganizationPlans.members`, `ORGANIZATION_CACHE_TTL_SECONDS` |
| `CachedRequest` | `is_member` with a fresh entry (`cached_member`) |
| `StartRead`, `FinishRead` | `hub_member` (an await on the hub), then `members.insert` for a member |
| `Allows` | current: `is_member` drops the role; fixed: `require_role(Member)` on the returned role |
| `Remove`, `ChangeRole`, `DeliverEvent` | `organization.membership_changed` and role events; today no public `evict` exists |
| `Tick` | the clock (fixed design: injectable) |

Fixed design: `evict(org, subject)` on every event, the role returned with membership, and an entry stored
by `FinishRead` only if no eviction ran on that replica since the hub read started.

## Invariants

- `NoGrantAfterTTL`: removed, and `now - t_remove > TTL`, implies not granted.
- `NoGrantAfterEvent`: once a removal or role event reached the replica, the old grant is gone.
- `NoGrantAfterRemovalEvent`: the same for removal alone.
- `BillingNeverGranted`: a Billing-role member never passes `require_role(Member)`.

## Configs

| Config | Expect |
|---|---|
| `MembershipCache.cfg` | pass |
| `MembershipCache_current.cfg` | violation of `BillingNeverGranted` |
| `MembershipCache_evict_current.cfg` | violation of `NoGrantAfterRemovalEvent` |
| `residual/NaiveEviction.cfg` | violation, not run in CI (see below) |

Replicas r1 and r2, `TTL = 2`, `MaxTime = 4`. Time does not pass while a hub read is in flight.

## Counterexamples on today's code

Role: the hub changes the member's role to Billing. Replica r1 misses its cache, reads the hub, sees a
member, and grants: `is_member` drops the role.

Eviction: r1 starts a hub read and sees a member. The admin removes the member and the event reaches r1,
which has no way to evict. The read finishes and caches the member, and every request for the next TTL is
granted.

## Design note for the fix

`residual/NaiveEviction.cfg` adds `evict` but stores the hub answer without the epoch check. The same trace
still fails: the read started before the eviction and re-caches the removed member for a full TTL after the
event. The fix needs a per-key generation (or a compare against an eviction counter) between the hub read
and the insert. Run it from this directory with `tlc2.TLC -deadlock -config residual/NaiveEviction.cfg
MembershipCache.tla`.

## Paired test

P-15 (organization plans with an injectable clock: after a removal event `is_member` is false; without an
event, false once the TTL passed; Billing never passes `require_role(Member)`).
