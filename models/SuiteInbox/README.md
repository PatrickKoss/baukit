# M-05 SuiteInbox

The hub-to-product platform event inbox: at-least-once redelivery from the hub, `prepare` outside the
transaction, request cancellation, and receipt purge after 30 days. No known finding.

## What maps to what

| Model | Code |
|---|---|
| `Send` | the hub delivering or redelivering an event; a changed body only for an id it already delivered (bodies are HMAC-signed, `platform/receiver.rs`) |
| `PreRead` | `PostgresPlatformInbox::receive` (`rust/crates/baukit-suite/src/platform/postgres.rs`): the receipt read outside any transaction; duplicate or `EventIdConflict` (409), else `handler.prepare` |
| `Commit` | the transaction: `INSERT ... ON CONFLICT DO NOTHING`, re-read on conflict, `handler.handle`, commit |
| `Cancel` | the request future dropped or `prepare` failing before commit |
| `Purge` | `purge_received_before` |
| `stateRev` | a revision-guarded product handler (applies an event only when its revision is newer) |

## Properties

- `ExactlyOneEffect`: the guarded handler changes product state at most once per event, also when the
  receipt was purged and the hub redelivers.
- `AckedImpliesApplied`: a 2xx for an event means its revision, or a newer one, is applied.
- `ChangedBodyIs409`: a changed body for a stored id is never acknowledged.
- `EveryEventAcked` (liveness, at-least-once): every event is eventually acknowledged. Fairness: the hub
  resends each unacked event under strong fairness (worker slots free up intermittently), each request in
  flight is processed under weak fairness, purge is unfair, and cancellation happens at most `MaxCancels`
  times, which stands in for "not forever".

A handler without a revision guard would apply an event twice after purge plus redelivery. The model
assumes the guard, as the plan's invariant does.

## Configs

| Config | Expect |
|---|---|
| `SuiteInbox.cfg` | pass |

Two events, two concurrent requests, `MaxCancels = 2`.

## Paired test

The platform inbox tests in `rust/crates/baukit-suite/tests` (duplicate, conflict, concurrent claims).
