# Offline readiness and sync-outcome contract

**Status:** Product-facing state contract; no shared sync protocol.
**Applies to:** Products that hydrate local data, pull remote data, or queue writes offline.
**Related:** [local-data ownership](./local-data-ownership-contract.md).

The contract standardizes what a product may claim to the user. It does not standardize revisions, conflict algorithms, transport payloads, storage engines, or repair workflows.

## 1. Readiness states

Expose an explicit initial-readiness state with these meanings, even if product code uses different names:

| State               | Meaning                                                                    | UI rule                                                        |
| ------------------- | -------------------------------------------------------------------------- | -------------------------------------------------------------- |
| `unknown`           | Local hydration or the required initial pull has not started.              | Show initialization, never an empty result.                    |
| `loading`           | Hydration/pull is running or a delivered snapshot is stale.                | Keep prior safe data if available; do not claim it is settled. |
| `settled-empty`     | Required hydration completed and the active partition contains no records. | A true empty state is now allowed.                             |
| `settled-populated` | Required hydration completed with records.                                 | Render the local records.                                      |
| `offline-cached`    | The network failed, but hydration completed and cached records are usable. | Render cached data and explain that local data remains safe.   |
| `blocked`           | Identity, corruption, migration, or another safety failure prevents use.   | Do not mount repositories or imply an empty dataset.           |

Never render `settled-empty` merely because a query returned no rows while readiness is `unknown` or `loading`.

## 2. Sync activity and outcomes

Keep sync activity separate from initial readiness. The user-visible activity vocabulary is:

- `idle`: no run active and no unsent or actionable work remains;
- `syncing`: a run is active;
- `pending`: unsent changes remain, including a retry scheduled after interruption;
- `attention`: at least one actionable rejection needs product/user action; and
- `transport-error`: the latest attempt failed without proving local data loss.

Classify each submitted mutation as `accepted`, `actionable-rejection`, or `superseded`. A superseded last-writer-wins outcome is benign only when the authoritative snapshot has already converged locally and no repair remains. Every other rejection is actionable unless the product proves another terminal, non-actionable classification.

Never display “synced” while unsent changes or actionable rejections remain. A completed HTTP request is not by itself a successful sync.

## 3. Time and failure semantics

- Record `lastAttemptAt` for the latest started or completed attempt and `lastSuccessAt` only when the product's success predicate is satisfied. Do not overwrite the latter on failure or partial acceptance.
- On network failure, keep committed local data and pending mutations. Explain retryability without exposing raw transport errors.
- A retry, timeout, cancellation, or process restart must not convert an unknown outcome into success. Reconcile or replay according to the product protocol.
- Reset readiness and sync state when the authenticated local-data partition changes.

## 4. Compound changes

Represent required parent/child writes as a dependency group, or stage the parent as incomplete until every required child is accepted. A parent must not appear complete while a required child is pending or rejected.

Repair and discard are explicit product-owned actions. The product defines edit routes, safe deletion scope, cascade rules, authorization, and copy; Baukit does not invent repair semantics or a universal sync engine.

## 5. Shared primitives

Baukit still does not standardize revisions, conflict algorithms, transport payloads, storage
engines, or repair workflows. It does ship the two mechanisms every product was rebuilding
underneath its own engine:

- `@baukit/sync-client` on the client: `SyncScheduler` (one run at a time, driven by an injected
  environment for timers, foreground state, and connectivity, with a single queued follow-up for
  writes that land mid-run), `SyncTransport` (injected `fetch` and per-attempt auth headers,
  failures typed as auth, partition mismatch, or retryable/non-retryable transport errors),
  `SyncStatusStore` (the section 2 vocabulary as an observable, React-free state, with
  `deriveLocalStoreReadiness` and `deriveInitialSyncState` for section 1), and `rankPushBatch`
  (orders a push batch parent-before-child, coalesces per entity, and holds back a parent whose
  required children are unsent, implementing section 4 without naming any entity).
- `baukit-sync` on the server: `next_revision`, which bumps a per-owner counter inside the
  caller's transaction so an allocation rolls back with its row write, plus the syncable-table
  column convention (`id`, `owner_id`, `updated_at`, `deleted_at`, `revision`, and an
  `(owner_id, revision)` index) shipped as reference migration SQL.

Both are mechanism, not protocol. Entity names, dependency order, endpoint paths, payload shapes,
and conflict rules remain product-owned.

## 6. Acceptance checks

- A cold start never flashes a settled empty state before hydration/pull is known.
- Cached data remains visible with an offline explanation when the initial network request fails.
- Unsent mutations produce `pending`; actionable rejections produce `attention`; neither path displays “synced.”
- A superseded mutation becomes non-actionable only after the authoritative state is present locally.
- `lastAttemptAt` advances on a failed run while `lastSuccessAt` remains unchanged.
- Network errors preserve local records and outbox rows.
- A parent with a rejected required child remains incomplete; repair can converge the whole group and discard follows product-defined atomic/cascade rules.
- The same outcome reducer is exercised for retry, timeout, cancellation, and restart/replay paths without prescribing a wire protocol.
