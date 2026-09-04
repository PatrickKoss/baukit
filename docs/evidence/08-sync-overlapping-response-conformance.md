# Overlapping sync-response conformance evidence

## Source product files

- `/home/patrick/projects/tiefgang/mobile/src/sync/rejected-push-race.test.ts`
- `/home/patrick/projects/tiefgang/mobile/src/db/repository-contract.ts`
- `/home/patrick/projects/tiefgang/mobile/src/db/dexie/sync-persistence.web.ts`
- `/home/patrick/projects/tiefgang/mobile/src/db/sqlite/sync-persistence.ts`
- `/home/patrick/projects/tiefgang/mobile/src/sync/engine.ts`

## Observed failure or repeated glue

Tiefgang reproduces a delayed rejected response that arrives after a newer settings write was
submitted. Its Dexie and SQLite stores delete only captured pending IDs, then check for a remaining
pending entity before applying the rejected server row. Both stores also skip stale pulls and move
the cursor in the same transaction.

## Baukit owner

`@baukit/sync-client/conformance` owns the storage-neutral race cases and adapter contract.

## Public types and errors

`SyncConformanceSubmittedBatchOutcome` carries the exact submitted rows, raw response, decoded
settlement, and rejected server rows. `SyncConformancePushResult` now accepts rejected rows.
`outbox.pendingId` exposes the stable pending-row identity to the harness. Missing atomic
settlement or identity mapping fails the case with a `Sync conformance failed` error.

## Product-owned inputs

Products keep entity payloads, wire outcomes, revision types, conflict choices, dependency ranks,
rejection details and copy, repair actions, SQL, and storage schemas.

## Cases

- Concurrency: accepted and rejected responses for submission A cannot remove or replace pending B.
- Failure: incomplete coverage settles nothing; staged pull failures roll back row and cursor.
- Privacy: fixtures use opaque values and errors contain no product payloads.
- Cleanup: each scenario disposes its clients and server through the existing adapter callback.

## Supported runtimes

ES2022 runtimes supported by the package. Adapters may use memory, browser storage, or native
storage if they can provide atomic transactions.

## Product adoption change

A Tiefgang adoption change will map `acknowledgePush` and `applyRejectedRows` to one conformance
outcome callback for both storage implementations. It can then remove the local overlapping cases
from `mobile/src/db/repository-contract.ts` and
`mobile/src/sync/rejected-push-race.test.ts` after those assertions run through Baukit's suite.
The product repository is read-only in this batch, so adoption has not run yet.
