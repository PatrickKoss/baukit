---
'@baukit/sync-client': patch
---

`SyncScheduler` retries failed runs when given `retry: { maxRetries, delayMs, onRetryScheduled? }`. `delayMs(error, retryIndex)` returns the wait in milliseconds or `null` for no retry; `fullJitterBackoffMs` from `@baukit/api-runtime/backoff` is the Baukit policy to pass in, so the root entry keeps no runtime dependencies. The retry index restarts after a success. Any trigger during the wait ends it and restarts the index: `trigger()`, the interval, foreground, or connectivity. `stop()` ends the wait without another run, and a wait that ends while backgrounded runs nothing. A bad delay or a throwing `delayMs` reaches `onError` and ends the retries. The constructor throws a `RangeError` for a `maxRetries` that is not a non-negative integer. New types: `SyncSchedulerRetryOptions` and `SyncSchedulerRetry`. Without `retry`, behavior is unchanged.

`trigger()` during a run no longer only joins it: when the run is waiting to retry, the wait ends. This applies only with `retry` set.
