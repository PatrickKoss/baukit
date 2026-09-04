# Browser sync environment evidence

## Source product files

- `/home/patrick/projects/redemut/packages/sync/src/scheduler-environments.ts`
- `/home/patrick/projects/redemut/packages/sync/test/scheduler-environments.test.ts`
- `/home/patrick/projects/redemut/web/src/account-screen.tsx`
- `/home/patrick/projects/redemut/mobile/src/account.tsx`

## Observed failure or repeated glue

Redemut supplies browser visibility, online-event, timer, cleanup, and retry-wake wiring around
Baukit's scheduler. Baukit already supplies the corresponding Expo environment.

## Baukit owner

`@baukit/sync-client/browser` owns browser host wiring. `SyncScheduler` owns the point where a host
recovery event can wake an engine-owned retry delay.

## Public types and errors

`createBrowserSyncEnvironment`, `BrowserSyncEnvironmentOptions`, `BrowserSyncDocument`,
`BrowserSyncWindow`, and `BrowserSyncTimers` form the browser entry. `SyncSchedulerOptions` adds
`onRecoverySignal`, which receives `active` or `online`. The factory throws specific missing-global
errors when called outside a browser without injected hosts.

## Product-owned inputs

Products keep the sync engine, retry policy, retry-delay implementation, run callback, interval,
analytics, query invalidation, and user-facing copy.

## Cases

- Concurrency: the scheduler still coalesces wake-triggered runs with an active run.
- Failure: imports under Node do not read DOM globals; factory calls without hosts fail explicitly.
- Privacy: events carry only `active` or `online`, with no URL, identity, or payload data.
- Cleanup: visibility and online subscriptions return idempotent cleanup functions.

## Supported runtimes

Current browsers with visibility, online, and interval APIs. Node 24 or newer may import the entry
and may call it with injected hosts.

## Retry wake-up decision

Retry wake-up belongs in `SyncSchedulerOptions.onRecoverySignal`, not in the environment. The
environment reports host state and cannot know whether the product engine is waiting in backoff.
The product callback may wake that delay before the scheduler joins or starts the run.

## Product adoption change

A Redemut adoption change will import `createBrowserSyncEnvironment` from Baukit, pass
`engine.wakeRetryDelay()` through `onRecoverySignal`, and delete
`packages/sync/src/scheduler-environments.ts` plus its copied environment tests. The product
repository is read-only in this batch, so adoption has not run yet.
