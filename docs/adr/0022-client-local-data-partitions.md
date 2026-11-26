# ADR 0022: Client local data lives in one partition per profile with explicit readiness states

## Status
Accepted, 2026-11-27.

## Context
Authenticated apps keep user data on the device. If two accounts share a store, or a late signal from one account acts on another, the wrong user reads or erases data before any server check runs.

Apps also need to tell "nothing synced yet" from "you have no data". An empty query during hydration looked like an empty account.

## Decision
`@baukit/data-contracts` derives each store name with `deriveScopedStoreName(namespace, subject)`, a SHA-256 of a length-prefixed canonical encoding, so the name does not disclose the profile ID. The partition key is the server profile ID from `/me`, passed as `subject`. `ScopedPersistenceLifecycle` holds one open partition with states `signed-out`, `initializing`, `ready` and `blocked`. `handleProfileErased(subject)`, `handleSessionExpired(subject)` and `blockIdentityMismatch(subject, error)` act only on the named subject. `recheckServerSubjectBeforeSyncAdoption` rechecks the server subject before pushing and aborts if the lifecycle moved. `WebStorageKeyValueStore` separates namespace and key with U+001F.

`@baukit/sync-client` exposes `InitialSyncState` (`unknown`, `syncing`, `offline-cached`, `sync-error-cached`, `settled`) from `deriveInitialSyncState`, and `LocalStoreReadiness`.

The rules live in the [local-data ownership contract](../platform/local-data-ownership-contract.md) and the [offline readiness contract](../platform/offline-readiness-contract.md).

## Enforced by
- TLA+ model [M-07 ClientAccountSwitch](../../models/ClientAccountSwitch/README.md): `ClientAccountSwitch.cfg` must pass and the three `_current.cfg` configs must fail. Gate step `models`.
- C-02 `keeps one open handle, honours the last request and erases only signalled subjects` in `typescript/packages/data-contracts/src/scoped-lifecycle.property.test.ts`.
- `fenced sync adoption` cases in `typescript/packages/data-contracts/src/sync-adoption.test.ts`, and `SyncStatusStore` cases in `typescript/packages/sync-client/src/store.test.ts`. Gate step `ts-check`.

## Consequences
Products get account isolation on the device and honest empty states. They must close and reopen storage on every account change and pass the subject to every signal.

## History
- 2026-11-25, `86f2ee8a`: profile-erased and expiry signals scoped to a subject. Before, they erased the last selected partition.
- 2026-11-25, `034edf19`: web storage keys moved from `namespace + key` to a U+001F delimiter.
