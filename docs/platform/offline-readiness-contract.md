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
| `sync-error-cached` | A non-network sync failure occurred, but cached records are usable.        | Render cached data without claiming the device is offline.     |
| `blocked`           | Identity, corruption, migration, or another safety failure prevents use.   | Do not mount repositories or imply an empty dataset.           |

Never render `settled-empty` merely because a query returned no rows while readiness is `unknown` or `loading`.

## 2. Sync activity and outcomes

Keep sync activity separate from initial readiness. The user-visible activity vocabulary is:

- `idle`: no run active and no unsent or actionable work remains;
- `syncing`: a run is active;
- `pending`: unsent changes remain, including a retry scheduled after interruption;
- `attention`: at least one actionable rejection needs product/user action; and
- `transport-error`: the latest attempt failed without proving local data loss.

Expose machine-readable failure metadata separately from activity. The shared failure kinds are
`auth`, `partition_mismatch`, `rate_limited`, `network`, `server`, `payload_compatibility`, and
`local_apply`. A rate-limit failure carries its retry time. A scheduled retry also exposes a
`retrying` indicator and retry time. Products supply any copy shown to users.

Classify each submitted mutation as `accepted`, `actionable-rejection`, or `superseded`. A superseded last-writer-wins outcome is benign only when the authoritative snapshot has already converged locally and no repair remains. Every other rejection is actionable unless the product proves another terminal, non-actionable classification.

Never display "synced" while unsent changes or actionable rejections remain. A completed HTTP request is not by itself a successful sync.

## 3. Time and failure semantics

- Record `lastAttemptAt` for the latest started or completed attempt and `lastSuccessAt` only when the product's success predicate is satisfied. Do not overwrite the latter on failure or partial acceptance.
- On network failure, keep committed local data and pending mutations. Explain retryability without exposing raw transport errors.
- A retry, timeout, cancellation, or process restart must not convert an unknown outcome into success. Reconcile or replay according to the product protocol.
- Apply a decoded push outcome in one local transaction. Settle only the pending IDs captured for
  that submission. A later local write to the same entity remains pending and visible when an
  older accepted or rejected response arrives.
- Reset readiness and sync state when the authenticated local-data partition changes.

`lastSyncAt` is a deprecated compatibility field for one release cycle. It always equals
`lastSuccessAt`. The snake-case projection follows the same rule: `last_sync_at` equals
`last_success_at`. Products must migrate persisted state to separate attempt and success fields.
When importing one old `lastSyncAt` value, use it as both timestamps because the old model cannot
recover the attempt time independently.

## 4. Tombstone horizons and full rebuilds

A server that purges tombstones must keep one purge horizon for each stable data owner. The
horizon is the greatest revision of any tombstone purged for that owner. Advancing it and deleting
the covered tombstones must commit atomically under the same owner serialization used for revision
allocation. A later purge may keep or raise the horizon, never lower it. Retention periods, tables,
foreign-key order, and owner keys remain product policy.

For a pull cursor `c` and owner horizon `h`:

- `c = 0` is an explicit full-rebuild request and the server must accept it;
- `0 < c < h` is stale because at least one deletion can no longer be replayed;
- `c = h` and `c > h` remain valid incremental requests; and
- deletion that is not part of sync history, such as age-based removal of pull-only telemetry, does
  not move the horizon.

A stale request returns HTTP 409 with this stable error data:

```json
{
  "error": {
    "code": "resync_required",
    "details": {
      "horizon_revision": 42
    }
  }
}
```

`horizon_revision` uses the same validated cursor representation as the pull endpoint. The client
must reject a missing, malformed, or non-advancing horizon as `payload_compatibility`. Human text,
owner identifiers, table names, and deleted row data do not belong in this response.

The client asks a product hook whether a disruptive reset is safe. The hook may defer while an
interactive operation, unsaved workflow, or other product-owned critical section is active. A
deferred reset leaves rows, cursor, pending mutations, and rejection records unchanged. The client
must not record sync success while it waits.

When the hook permits reset, one local transaction must:

1. remove server-backed rows that have no pending mutation or explicit rejection;
2. preserve durable pending mutations and their visible local rows;
3. preserve explicit rejection records and the rows needed to repair them; and
4. set the pull cursor to zero.

The deletion order must satisfy the product schema. Child rows may need removal before parents.
Pull-only rows are server-backed and must be cleared even though they never enter the outbox. A
failure after any staged deletion rolls back every row change and the cursor update.

After reset, the client reconciles retained mutations and requests a complete snapshot with cursor
zero. It may push pending mutations before that pull or overlay them during page application. In
both cases, a pulled row must not replace a newer pending local edit. Pull pages and their cursors
still commit atomically under the rules in section 3.

A second `resync_required` response to the immediate cursor-zero request violates the server
contract. Stop the run after that response. Do not reset again or loop. Keep pending mutations and
rejection records, leave the cursor at zero, and report a `payload_compatibility` failure for product
recovery.

## 5. Compound changes

Represent required parent/child writes as a dependency group, or stage the parent as incomplete until every required child is accepted. A parent must not appear complete while a required child is pending or rejected.

Repair and discard are explicit product-owned actions. The product defines edit routes, safe deletion scope, cascade rules, authorization, and copy; Baukit does not invent repair semantics or a universal sync engine.

## 6. Shared primitives

Baukit still does not standardize revisions, conflict algorithms, transport payloads, storage
engines, or repair workflows. It does ship the two mechanisms every product was rebuilding
underneath its own engine:

- `@baukit/sync-client` on the client: `SyncScheduler` runs one callback at a time and queues one
  follow-up for writes made during a run. `SyncTransport` injects `fetch` and per-attempt auth
  headers, classifies failures, and preserves `Retry-After` on 429 responses. `SyncStatusStore`
  exposes attempt and success times, typed failure metadata, retry state, pending work, and
  readiness helpers. `rankPushBatch` orders and coalesces product-owned entities. Pull-page and
  push-outcome validators reject payloads that could skip work or loop without progress. The
  `@baukit/sync-client/conformance` entry runs the shared failure and convergence cases against
  product callbacks without choosing the product's protocol. The
  `@baukit/sync-client/browser` entry supplies visibility, online-event, and timer wiring without
  reading browser globals during module evaluation.
- `baukit-sync` on the server: `next_revision`, which bumps a per-owner counter inside the
  caller's transaction so an allocation rolls back with its row write, plus the syncable-table
  column convention (`id`, `owner_id`, `updated_at`, `deleted_at`, `revision`, and an
  `(owner_id, revision)` index) shipped as reference migration SQL.

Both are mechanism, not protocol. Entity names, dependency order, endpoint paths, payload shapes,
and conflict rules remain product-owned.

## 7. Product conformance gate

Register every case returned by `createSyncConformanceTests` in the product's normal Vitest or Jest
suite. Build an isolated scenario for each case with two local clients and one fake server. Supply
these callbacks:

- outbox enqueue and pending listing;
- atomic submitted-batch outcome application, including exact pending IDs, decoded rejected server
  rows, actionable-rejection storage, and product-owned accepted-revision stamping;
- local cursor reads, full snapshots including tombstones, pending-state reads, and atomic page
  application with injected failure;
- push encoding, push-outcome decoding, pull-page decoding, and cursor comparison; and
- fake-server push, paged pull, seeding, snapshots, incomplete outcomes, cursor faults, and network
  and server failures.

Products with finite tombstone retention also supply the optional full-resync callbacks. They map
the product's stale-cursor error onto `SyncConformanceStaleCursor`, expose cursor zero, run the
product safety hook and atomic reset, and let the fake server purge named fixture rows or reject one
cursor-zero request. Products without finite retention omit these callbacks and keep the existing
case set.

Map the harness fixtures to product entities and payloads at the adapter boundary. Use the same
wire codecs, outbox operations, transaction code, cursor store, and pending-state derivation as
the production sync path. Keep the fake deterministic. An injected local failure after one staged
row must roll back the whole page and its cursor.

The submitted-batch callback must use one storage transaction. An older response may settle only
the pending IDs captured before the request. It must not delete a later pending row or replace that
row's visible payload. Products choose their revision type and conflict algorithm. If a product
stamps an accepted server revision onto local metadata, the stamp must not overwrite the payload
of a newer local write.

Partition each scenario by the same stable identity key used in production. A client wired to one
partition must never read another partition's outbox, cursor, rejection log, or local rows.

## 8. Acceptance checks

- A cold start never flashes a settled empty state before hydration/pull is known.
- Cached data remains visible with an offline explanation when the initial network request fails.
- Unsent mutations produce `pending`; actionable rejections produce `attention`; neither path displays "synced."
- A superseded mutation becomes non-actionable only after the authoritative state is present locally.
- `lastAttemptAt` advances on a failed run while `lastSuccessAt` remains unchanged.
- Network errors preserve local records and outbox rows.
- Server and local-apply failures also preserve the pending count.
- A 429 remains `rate_limited`, carries the resolved retry time, and does not become a generic
  offline state.
- A successful push response has an outcome for every submitted entity before any outcome is
  acknowledged.
- Late accepted and rejected responses settle only their submitted pending IDs. A newer local write
  to the same entity stays pending and visible.
- An older or equal pulled revision cannot replace a pending local row, but the page and cursor
  still commit together.
- A pull cursor never regresses, and `has_more` requires cursor progress.
- The stored pull cursor changes only after the local transaction succeeds.
- A per-owner purge horizon never regresses. Cursor zero bypasses it, while a stale nonzero cursor
  returns `resync_required` with `details.horizon_revision`.
- An unsafe reset is deferred without changing local state or recording success.
- A reset clears parent, child, and pull-only server rows in one transaction with cursor zero. It
  preserves pending edits, explicit rejection records, and the rows needed to resolve them.
- A failed reset commits neither row deletion nor cursor zero.
- A second stale-cursor response immediately after reset stops the run without another reset.
- Every product sync suite passes all cases from `createSyncConformanceTests` against its own
  adapters and fake server.
- A parent with a rejected required child remains incomplete; repair can converge the whole group and discard follows product-defined atomic/cascade rules.
- The same outcome reducer is exercised for retry, timeout, cancellation, and restart/replay paths without prescribing a wire protocol.
