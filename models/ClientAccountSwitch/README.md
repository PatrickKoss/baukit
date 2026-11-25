# M-07 ClientAccountSwitch

One device with several tabs sharing a session token, the lifecycle of the tab under test, a partition with
an outbox per subject, server-side erasure, and this device's own erasure flow. Settles F-38, F-53 and F-54.

## What maps to what

| Model | Code (`typescript/packages`) |
|---|---|
| `SignIn`, `SignOut` | a sign-in or sign-out in any tab; the token changes now and the broadcast arrives later |
| `Deliver` | the tab applying the broadcast: `ScopedPersistenceLifecycle.selectSubject` or `clear` (`data-contracts/src/scoped-lifecycle.ts`) |
| `Request`, `Respond` | `api-runtime/src/fetch.ts`; a 401 `profile_erased` fires `onProfileErased`, the template calls `handleProfileErased` and `clearSession` |
| `handleProfileErased` (current) | erases `lastSelectedPartition`, with no subject and no session check |
| `Recheck`, `Push` | `recheckServerSubjectBeforeSyncAdoption` then `adopt()` (`data-contracts/src/identity.ts`); the push reads the token at send time |
| `EraseStart`, `EraseServerCall`, `EraseLocalDelete` | `data-contracts/src/erasure.ts`: stop sync, server erase, local delete; today the erased flag lives in memory |
| `Reload` | a tab reload or crash: memory is lost, storage stays |
| `EraseServerElsewhere` | erasure from another device or the data-subject API |

Fixed design: `handleProfileErased(subject)` erases only that subject's partition; api-runtime stamps each
request with the session generation and drops signals from an older one; adopt is bound to the generation
at recheck, so a switch in between aborts the push; a durable erased marker is written before the server
call, cleared after the local delete, and blocks reopening the partition.

The recheck in the model compares the token with the partition's subject on the client. The real recheck
also asks the server, whose fence would reject an erased subject's token, so the F-54 trace below shows
data leaving the device's control (the partition reopens and tries to sync) rather than the server
accepting it.

## Invariants

- `PushCarriesOwnToken`: every push from partition p carries subject(p)'s token (F-53).
- `EraseTouchesOnlyErased`: a `profile_erased` signal only deletes partitions of erased subjects (F-38).
- `NoPushAfterErase`: no push for s after this device erased s on the server (F-54).

## Configs

| Config | Expect |
|---|---|
| `ClientAccountSwitch.cfg` | pass, all three invariants |
| `ClientAccountSwitch_current.cfg` | violation of `EraseTouchesOnlyErased` (F-38) |
| `ClientAccountSwitch_adopt_current.cfg` | violation of `PushCarriesOwnToken` (F-53) |
| `ClientAccountSwitch_erasure_current.cfg` | violation of `NoPushAfterErase` (F-54) |

Subjects A and B, at most 3 session generations.

## Counterexamples on today's code

F-38: A signs in. A's profile is erased on another device. A request with A's token goes out. B signs in
in another tab and tab 1 selects B's partition. The stale 401 `profile_erased` lands and
`handleProfileErased` deletes `lastSelectedPartition`, which is now B's.

F-53: A signs in, edits, and the recheck confirms A. Before `adopt` pushes, a sign-in in another tab
replaces the shared token. The push reads the token at send time and sends A's outbox with the other
session's token.

F-54: A starts erasing its own profile and the server erase succeeds. The tab reloads before the local
delete. The in-memory erased flag is gone, the session still names A, the partition reopens, and the outbox
is pushed again.

## Paired test

C-02 (fast-check `fc.commands` with `fc.scheduler` over `ScopedPersistenceLifecycle` and `InMemoryStorePool`).
