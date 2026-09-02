---
name: baukit-offline-sync
description: Add, change, or review product-owned offline sync using Baukit's client primitives and conformance harness. Use for outboxes, push and pull loops, cursors, tombstones, replay, local transactions, sync status, identity partitions, or multi-client convergence.
---

# Build product offline sync

Read `<baukit-repo>/docs/platform/offline-readiness-contract.md` and the
`@baukit/sync-client` README before changing product code. Keep entity names, payload schemas,
endpoint paths, conflict policy, storage, identity, and copy in the product.

## Partition every local store

Derive one stable local-data partition from authenticated identity. Store the outbox, pull cursor,
rejection log, and synced rows inside that partition. Reset readiness before switching identity.
Block access if the server rejects the retained partition. Never adopt a replacement identity into
an old local database.

## Keep pending state honest

Derive the pending count from durable outbox rows. Do not clear a row after sending it or receiving
an HTTP success. Decode the full response and validate one outcome per submitted entity first.
Then acknowledge accepted and terminally rejected rows, and retain actionable rejections for the
repair UI. Network, server, cancellation, and local-apply failures leave uncertain work pending.

## Guard pull progress

Treat cursors as opaque product values and supply an explicit comparator. Reject a cursor that
moves backwards. Reject `has_more` when the cursor did not advance. Apply a decoded page and its
next cursor in one local transaction. Inject a failure after at least one staged row and prove the
transaction leaves both data and cursor unchanged.

## Make replay and deletion converge

Give each local mutation a stable replay identity. The server must accept a repeated request
without creating another row or revision. Keep tombstones in pull history long enough for every
supported client to observe deletion. Test two clients with independent outboxes, alternate pushes
and pulls, add a tombstone, and compare their complete local snapshots with the server snapshot.

Order referenced records before dependent records in each push batch. If a required child is
rejected, keep its parent incomplete or hold the parent back according to product policy. Do not
hide the rejection or report the group as synced.

## Wire the conformance harness

Import `createSyncConformanceTests` from `@baukit/sync-client/conformance`. Implement its adapter
with the product's real outbox operations, transaction and cursor code, pending-state reader, and
wire codecs. Supply a deterministic fake server with paged pulls and the requested fault controls.
Create two fresh clients in the same identity partition for every case.

Register each returned `{ name, run }` case with Vitest or Jest. Do not copy the cases into the
product. Run the full product format, typecheck, lint, unit, and integration gates after the
conformance cases pass.
