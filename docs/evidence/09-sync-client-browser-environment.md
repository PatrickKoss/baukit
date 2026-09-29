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

## Follow-up 0.5.1 (2026-09-29)

Source revisions: Baukit baseline 28db260, Redemut 1d40e90. The sync-client README cites no
evidence note. This note gets the follow-up because it is the scheduler note and it recorded the
earlier decision to keep retry in the product. Note 45 covers server purge horizons and says
nothing about the scheduler.

### Product evidence

`SyncScheduler` passed a failed run to `onError` and then waited for the next trigger. The only
Baukit backoff was the private `retryDelay` in `@baukit/api-runtime`. Redemut therefore keeps a
second retry loop in `packages/sync/src/sync-engine.ts`: `#retryDelay` (line 669, equal jitter),
`#waitForRetry`, `#retryWake`, `#retryResetRequested`, `retryNow`, `wakeRetryDelay`, and the
options `maxAttempts`, `baseDelayMs`, `maxDelayMs`, `sleep`, and `random`. Its schedulers in
`web/src/account-screen.tsx:938` and `mobile/src/account.tsx:218` wire `retryNow` and
`wakeRetryDelay` back into the scheduler by hand.

### Decisions

- Retry moves into the scheduler as `SyncSchedulerOptions.retry: { maxRetries, delayMs,
  onRetryScheduled? }`. This replaces the earlier "retry is product-owned" decision above. The
  scheduler already owns the triggers that should end a wait (foreground, connectivity, the
  interval, and `trigger()`), so the wake-up plumbing Redemut wrote by hand becomes internal. The
  retry index restarts after a success or a wake-up. `stop()` ends a wait without another run,
  and a wait that ends while the app is in the background runs nothing.
- The delay formula is shared, not copied. `@baukit/api-runtime/backoff` exports
  `fullJitterBackoffMs(retryIndex, { baseDelayMs, maxDelayMs, random? })`. It is the full-jitter
  formula the private `retryDelay` already used, moved out unchanged, and the request retries now
  call it. It is a subpath with no imports, so it does not pull the api-runtime client into a
  bundle.
- The scheduler takes the delay as a function, `delayMs(error, retryIndex)`, instead of importing
  the policy. The sync-client README promises a root entry with no runtime dependencies, and the
  function also lets a product return `null` for a failure it must not retry, such as an auth
  error.
- The environment has interval timers only, so the retry wait is one interval cleared on its
  first tick. No new host capability was added.

### Gates

TypeScript workspace, run from `typescript/`: `build`, `format:check`, `lint`, `test`, and `check`
all passed. `scheduler-retry.test.ts` has 11 tests: delays until success, the `maxRetries` limit,
a `null` delay, the error passed to `delayMs`, a trigger during the wait, foreground and
connectivity wake-ups, backgrounding during the wait, `stop()` during the wait, a follow-up during
the wait, a throwing or invalid `delayMs`, and `maxRetries` validation. The existing scheduler
tests pass unchanged.

### Breaks

- With `retry` set, `trigger()` ends a retry wait. Without `retry`, the scheduler behaves as in
  0.5.0.
- Redemut's move from equal to full jitter changes behavior: a retry can now start at any point
  between 0 and the capped delay, where equal jitter waited at least half of it.

### Product adoption

Redemut: delete the retry loop and delay code listed above from `packages/sync/src/sync-engine.ts`.
In `web/src/account-screen.tsx` and `mobile/src/account.tsx`, pass
`retry: { maxRetries, delayMs: (_error, index) => fullJitterBackoffMs(index, policy), onRetryScheduled }`
to `SyncScheduler`, and publish retry status from `onRetryScheduled`. Both apps already depend on
`@baukit/api-runtime`; `packages/sync` depends only on `@baukit/sync-client`, so the policy is
passed in from the apps rather than added as a dependency there.
