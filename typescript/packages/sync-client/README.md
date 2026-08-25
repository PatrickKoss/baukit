# @baukit/sync-client

`@baukit/sync-client` holds the four client-side pieces every offline-capable product rebuilds
around a sync loop: when to run, how to send a request, how to report status, and what order to
push a batch in. It does not define a sync protocol. Endpoint paths, payloads, conflict rules, and
the sync engine itself stay product-owned, as
[the offline readiness contract](../../../docs/platform/offline-readiness-contract.md) says they
must.

The root entry has no runtime dependencies and no React. Timers, foreground state, connectivity,
and `fetch` all arrive by injection, so the same code runs under Node, a browser, and React Native.
The optional `@baukit/sync-client/expo` entry imports `react-native` and `expo-network`; neither is
loaded by the root entry.

## Scheduler

`SyncScheduler` takes one opaque `run()` callback and a `SyncSchedulerEnvironment`. It guarantees
a single active run: a trigger arriving mid-run joins that run instead of starting a second one.

```ts
const scheduler = new SyncScheduler(() => engine.run(), environment, { intervalMs: 60_000 });
scheduler.start();
```

`trigger()` starts a run or joins the active one. `requestFollowUp()` is what a local write calls:
if a run is already active it queues exactly one replay after that run settles, so a write made
while the engine was mid-push is never left unsent. Repeated follow-up requests still collapse
into one replay. `stop()` drops the queued replay along with the interval and subscriptions.

The environment reports foreground state and connectivity. The scheduler runs on start, when the
app returns to the foreground, when connectivity returns, and on the interval. It installs no
interval while backgrounded and ignores connectivity events there.

A failing run reaches `onError` and does not stop scheduling. Backoff between attempts belongs to
the product's engine, inside `run()`, where it can see which failures are retryable.

### Expo environment

Expo products can use the supplied environment instead of repeating AppState and network wiring:

```ts
import { SyncScheduler } from '@baukit/sync-client';
import { createExpoSyncEnvironment } from '@baukit/sync-client/expo';

const scheduler = new SyncScheduler(() => engine.run(), createExpoSyncEnvironment());
```

The environment treats only AppState `active` as foreground. On native platforms it reads the
initial state from `expo-network` and triggers only when connectivity changes from unusable to
usable. Expo web uses the browser `online` event. Tests can pass `{ timers }` to replace
`setInterval` and `clearInterval`. `react-native` and `expo-network` are optional peers because
non-Expo consumers never import this subpath.

## Transport

The original `SyncTransport` constructor wraps an injected `fetch` and auth header provider, and
turns every failure into a typed error:

- `SyncAuthError` for 401, never retryable;
- `SyncPartitionMismatchError` when the body carries the configured partition-mismatch code,
  meaning the local database belongs to an erased or replaced owner;
- `SyncTransportError` otherwise, with `retryable` true for 408, 429, and 5xx, plus network and
  body-read failures.

Auth headers resolve per attempt, so a token refreshed between retries is picked up. The
partition header is sent only when the caller passes a `partitionId`, and both its name and the
mismatch code are configurable. The transport never names an endpoint: callers pass the path and
the response type.

A product API client often already owns the base URL, token refresh, authentication recovery,
response decoding, and its error policy. Pass its decoded request function instead of duplicating
that work in the transport:

```ts
const transport = new SyncTransport({
  request: apiClient.request.bind(apiClient),
  partitionHeader: 'X-Product-Profile-Id',
});

await transport.request('/api/v1/sync/push', {
  method: 'POST',
  body: { changes },
  partitionId: profileId,
});
```

This mode builds the relative path, query string, JSON body, and partition header, then delegates.
The request function owns decoding and errors, which keeps the product API client's existing auth
recovery intact. The fetch-backed constructor remains unchanged for products that want
`SyncTransportError` classification from response status and body.

## Status store

`SyncStatusStore` is the observable sync status, with `subscribe`/`getSnapshot` and no state
library. It encodes rules the offline readiness contract requires and products keep getting
wrong:

- an actionable rejection outranks a transport failure, so a failed run over existing conflicts
  still reads `attention`;
- unsent work outranks a bare error, so a failed run with pending changes reads `pending`,
  not `error`;
- the first run is the initial pull; later runs are settled activity.

`refreshRevision` increments whenever a run delivers a new authoritative snapshot.
`deriveLocalStoreReadiness` compares it against what a consumer has actually rendered, which is
how a cold start avoids flashing an empty state over data that is still arriving.
`deriveInitialSyncState` maps a snapshot onto the contract's `unknown` / `syncing` /
`offline-cached` / `settled` vocabulary.

The store is generic over its attention item and defaults to the existing
`{ entityType, entityId }` shape. This keeps one store API while allowing a product to retain the
rejection details its UI needs:

```ts
interface ProductAttention {
  object_entity_type: string;
  object_entity_id: string;
  rejections: readonly Rejection[];
}

const status = new SyncStatusStore<ProductAttention>();
status.setAttention(items, pendingCount);
```

For stores and screens that use API-style names, `toSnakeCaseSnapshot` returns a typed projection:

```ts
const snapshot = toSnakeCaseSnapshot(status.getSnapshot());

snapshot.last_sync_at;
snapshot.pending_count;
snapshot.initial_pull_status;
snapshot.refresh_revision;
snapshot.security_block;
```

`status`, `error`, and `attention` keep their names. Attention items also keep their original
shape.

## Push batch ranking

`rankPushBatch` orders one push batch so a parent is never sent after its children.

```ts
const rank = dependencyRankByOrder(['container', 'group', 'leaf']);
const batch = rankPushBatch(pending, {
  rank,
  batchSize: 200,
  isHeldBack: (change) => hasUnsentChildren(change),
});
```

Changes are grouped by `entityType:entityId`. Each group keeps the position of its first
occurrence, so foreign-key order survives coalescing, and carries the newest change so the
request sends current data. `coveredChangeIds` lists every queued change the group settles,
because server outcomes are keyed by entity rather than by outbox row: one outcome clears all of
them.

`isHeldBack` is the other half of the rule. A parent whose required children are still unsent is
dropped from the batch rather than reordered, and retried on a later run once those children have
been accepted. Sending it now would show the server a complete parent while a required child is
missing. Held-back entities do not consume batch-size slots.

Entity names, the dependency order, and the hold-back predicate are all product inputs. Baukit
knows only that a rank exists.
