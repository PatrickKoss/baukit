# `@baukit/sync-client`

`@baukit/sync-client` holds the four client-side pieces every offline-capable product rebuilds
around a sync loop: when to run, how to send a request, how to report status, and what order to
push a batch in. It does not define a sync protocol. Endpoint paths, payloads, conflict rules, and
the sync engine itself stay product-owned, as
[the offline readiness contract](../../../docs/platform/offline-readiness-contract.md) says they
must.

The root entry has no runtime dependencies and no React. Timers, foreground state, connectivity,
and `fetch` all arrive by injection, so the same code runs under Node, a browser, and React Native.
The optional `@baukit/sync-client/expo` entry imports `react-native` and `expo-network`; neither is
loaded by the root entry. The `@baukit/sync-client/browser` entry reads browser globals only when
`createBrowserSyncEnvironment` is called, so importing it under Node is safe.

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

If the engine waits inside its own retry delay, pass `onRecoverySignal` to the scheduler and wake
that delay there. The scheduler calls it with `active` before a foreground-triggered run and with
`online` before an online-triggered run. Online recovery is reported even while the scheduler is
backgrounded, although the scheduler still waits for foreground before starting another run.
Retry wake-up belongs in this scheduler option, not in a decorated environment, because the
environment does not know whether an engine has a retry delay.

```ts
const scheduler = new SyncScheduler(() => engine.retryNow(), environment, {
  onRecoverySignal: () => engine.wakeRetryDelay(),
});
```

### Browser environment

Browser products can import an environment that maps document visibility, online events, and
intervals to the scheduler contract:

```ts
import { SyncScheduler } from '@baukit/sync-client';
import { createBrowserSyncEnvironment } from '@baukit/sync-client/browser';

const scheduler = new SyncScheduler(
  () => engine.retryNow(),
  createBrowserSyncEnvironment(),
  { onRecoverySignal: () => engine.wakeRetryDelay() },
);
```

`isActive` is true only while `document.visibilityState` is `visible`. A visibility change reports
the new active state, and an `online` event reports a connectivity wake signal. Each returned
cleanup function may be called more than once and removes its listener once.

Tests and non-browser hosts can inject `{ document, window, timers }`. Calling the factory without
injections outside a browser throws a specific missing-document or missing-window error. Importing
the entry never reads either global. The browser entry has no React, Dexie, or sync-engine import.

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

The fetch-backed `SyncTransport` constructor wraps an injected `fetch` and auth header provider.
It classifies failures as follows:

- `SyncAuthError` for 401, never retryable
- `SyncPartitionMismatchError` when the body carries the configured partition-mismatch code
- `SyncRateLimitError` for 429, always retryable and carrying `retryAt` plus the raw `retryAfter`
  header
- `SyncNetworkError` when `fetch` or response-body reading fails, always retryable
- `SyncServerError` for other HTTP failures, retryable for 408 and 5xx
- `SyncPayloadCompatibilityError` when a successful response is not valid JSON, never retryable

`Retry-After` accepts integer delta seconds and HTTP dates. Missing, invalid, negative, and past
values use `now + retryAfterFallbackMs`. The default fallback is 60 seconds. Tests and products
with another policy can inject `now` and `retryAfterFallbackMs` in the fetch-backed constructor.

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
recovery intact. Use the fetch-backed constructor when the product wants package-owned response
classification.

## Status store

`SyncStatusStore` is the observable sync status, with `subscribe`/`getSnapshot` and no state
library. It encodes rules the offline readiness contract requires and products keep getting
wrong:

- an actionable rejection outranks a transport failure, so a failed run over existing conflicts
  still reads `attention`;
- unsent work outranks a bare error, so a failed run with pending changes reads `pending`,
  not `error`;
- the first run is the initial pull; later runs are settled activity;
- a failure updates `lastAttemptAt` without changing `lastSuccessAt` or clearing `pendingCount`.

Call `setSyncing(attemptAt)` when an attempt starts. `setIdle(successAt)` records a successful
attempt and clears work only after the product's success predicate passes. `setFailure` accepts
machine-readable metadata with one of these kinds: `auth`, `partition_mismatch`, `rate_limited`,
`network`, `server`, `payload_compatibility`, or `local_apply`. Product copy remains in the
separate `error` field.

Rate-limit metadata contains `retryAt`. Other retryable failures can pass `retryAt` in the
`SyncFailureUpdate` argument or call `setRetrying(retryAt)`. Snapshots expose both `retrying` and
`retryAt`, so a screen can distinguish scheduled retries and rate limits from an offline state.

`refreshRevision` increments whenever a run delivers a new authoritative snapshot.
`deriveLocalStoreReadiness` compares it against what a consumer has actually rendered, which is
how a cold start avoids flashing an empty state over data that is still arriving.
`deriveInitialSyncState` maps a snapshot onto `unknown`, `syncing`, `offline-cached`,
`sync-error-cached`, or `settled`. Only a first-run `network` failure maps to `offline-cached`.

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

snapshot.last_attempt_at;
snapshot.last_success_at;
snapshot.last_sync_at;
snapshot.failure;
snapshot.retrying;
snapshot.retry_at;
snapshot.pending_count;
snapshot.initial_pull_status;
snapshot.refresh_revision;
snapshot.security_block;
```

`status`, `error`, and `attention` keep their names. Attention items also keep their original
shape.

### Timestamp migration

`lastSyncAt` and `last_sync_at` are deprecated compatibility fields for one release cycle. They
are derived from `lastSuccessAt` and `last_success_at`. New code must not write or persist them as
attempt timestamps.

Replace `snapshot.lastSyncAt` with `snapshot.lastSuccessAt`. Replace `snapshot.last_sync_at` with
`snapshot.last_success_at`. Persist both attempt and success timestamps, then hydrate with an
object:

```ts
status.hydrate({
  lastAttemptAt: persisted.lastAttemptAt,
  lastSuccessAt: persisted.lastSuccessAt,
  attention: persisted.attention,
  pendingCount: persisted.pendingCount,
});
```

The deprecated `hydrate(lastSyncAt, attention, pendingCount)` overload treats the old value as
both the last attempt and last success. Remove that overload call when the persisted schema has
both fields.

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

## Payload validators

Call `validatePushOutcomeCoverage` before acknowledging any submitted change. Pass one key reader
for submitted entities and one for outcomes, then combine all accepted and rejected outcomes in
the second argument. The function throws `SyncPayloadCompatibilityError` if any submitted entity
has no outcome.

Call `validatePullPage(currentCursor, page, compare)` before following `hasMore`. Cursors are
opaque. The injected comparator defines their order. A regressing cursor, a non-finite comparison,
or `hasMore` without cursor progress throws `SyncPayloadCompatibilityError`.

`commitCursorAfterLocalTransaction` runs the supplied local transaction first. It invokes
`commitCursor` only after that promise resolves, and wraps local failures in `SyncLocalApplyError`.
This ordering prevents a failed local apply from skipping remote changes on the next pull.

## Conformance harness

`@baukit/sync-client/conformance` exports `createSyncConformanceTests`. It returns plain async test
cases and has no Vitest or Jest dependency. Register the returned cases with the runner already in
the product:

```ts
import { createSyncConformanceTests } from '@baukit/sync-client/conformance';

describe('product sync contract', () => {
  for (const testCase of createSyncConformanceTests(productAdapter)) {
    it(testCase.name, testCase.run);
  }
});
```

The adapter supplies two isolated clients and one fake server per case. It also supplies callbacks
for outbox enqueue, listing, and stable pending IDs; atomic submitted-batch settlement; atomic pull
application and cursor storage; pending-state reporting; wire encoding and decoding; and
fake-server push, pull, seeding, snapshots, and fault injection. Canonical fixture changes contain
opaque entity names, values, logical times, dependencies, and tombstones. The adapter maps them to
the product's schema and payloads.

The cases cover replay, transaction rollback, paged cursor progress, stalled and regressing
cursors, complete push outcomes, late accepted and rejected responses, stale pulls over pending
writes, pending state after failures, dependency-safe push order, and two-client convergence with
a tombstone. `wire.decodePush` must call `validatePushOutcomeCoverage` before returning
acknowledged rows. It returns decoded `rejectedRows` when a rejection contains an authoritative
server row.

`local.applySubmittedBatchOutcome` receives the exact `submitted` rows, the raw push `response`,
the decoded acknowledged rows and rejections, and any decoded rejected server rows. In one storage
transaction it must settle only pending IDs covered by that submission, record actionable
rejections, apply safe rejected rows, and perform any accepted-revision stamping. A newer pending
row for the same entity blocks an older response from replacing its local payload. Revision
stamping is product policy. The harness does not require last-writer-wins or a particular revision
type, but a stamp from an older submission must not replace the payload of a newer local write.

The pull callback must keep a pending local row when an older or equal remote revision arrives,
while committing the page cursor in the same transaction. Its injected failure must roll back both
the staged row and cursor.

### Conformance adapter migration

Existing adapter objects still type-check because `applySubmittedBatchOutcome` and `pendingId` are
optional, and the old `markAcknowledged` and `recordRejected` callbacks remain in the interface.
The expanded suite stops with a conformance error until the two new callbacks are present.
Implement atomic settlement with the same transaction used by the production sync path, return
rejected server rows from `decodePush`, then remove split settlement calls from the product engine.
Do not emulate the new callback by calling the two old callbacks in sequence.
