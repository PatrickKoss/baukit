# baukit-resilience

`baukit-resilience` holds the retry and failure-handling logic that the egress
client, the job runner and the auth caches share. It has no HTTP, database or
runtime dependency. Time comes from a `Clock` port and randomness from a
`JitterSource` port, so every decision can be replayed in a test.

```rust,ignore
use baukit_resilience::{Admission, BreakerPolicy, CircuitBreaker, Outcome};

let breaker = CircuitBreaker::new(BreakerPolicy::default());
let permit = match breaker.admit() {
    Admission::Allowed(permit) | Admission::Probe(permit) => permit,
    Admission::Rejected { retry_after } => return Err(Unavailable { retry_after }),
};
let outcome = match call().await {
    Ok(_) => Outcome::Success,
    Err(error) if error.is_timeout() => Outcome::Timeout,
    Err(_) => Outcome::Failure,
};
breaker.record(permit, outcome);
```

## Modules

| Module | What it does |
|---|---|
| `clock` | `Clock` port, `SystemClock`, and `ManualClock` behind the `test-support` feature. |
| `backoff` | `Backoff::delay(attempt, fraction)` with full jitter (default) or a stateless decorrelated jitter. Pure, whole milliseconds. |
| `breaker` | `BreakerMachine` (pure), `CircuitBreaker` (mutex and clock around it), `BreakerPolicy`, `Outcome`, `Admission`, `BreakerState`. |
| `breaker_set` | `BreakerSet<K>`: one breaker per key, at most 256 keys, evicting the longest-idle closed key first. `worst_state()` feeds a gauge. |
| `deadline` | `Deadline::cap(timeout, now)` shortens a timeout to the budget left. With the `task-deadline` feature, `deadline::scope(deadline, future)` sets a task-local deadline (a nested scope keeps the earlier one) and `deadline::current()` reads it. |
| `startup` | With the `startup` feature, `retry_within(dependency, budget, is_transient, on_retry, attempt)` retries until success, a permanent error (`StartupError::Permanent`) or the end of the budget (`StartupError::BudgetExhausted`, with the attempt count and last error). `on_retry` gets each `RetryAttempt` so the caller can log it. baukit-ops, baukit-ratelimit and baukit-auth use it at startup. |
| `stale` | `StaleCache<T>`: fresh TTL, stale-on-error window, failure cooldown and single flight. Uses `tokio` (`sync` only). |

## Breaker policy

`BreakerPolicy::default()` keeps the last 20 calls. It opens at a 50% failure
rate once 10 calls are in the window, or after 5 consecutive timeouts. The
first opening lasts 30 s, each re-open from half-open doubles it up to 5 min,
and up to 20% is taken off as jitter. Half-open admits one probe: a success
closes the breaker, a failure or timeout opens it again. A probe that reports
nothing within 30 s is abandoned and the next call becomes the probe.

Every outcome is recorded with the `Permit` its admission returned. A call
admitted while closed that finishes after the breaker opened is ignored, and
so is a probe that was replaced after its timeout. Only the current probe
decides half-open. `HoldFor` from any permit can still extend an open breaker.

`Outcome::HoldFor(delay)` is for an upstream `Retry-After`. It opens the
breaker for that delay, capped at 5 min, without counting as a failure.
`Outcome::Ignored` covers results that say nothing about the destination, such
as a `400`; from the probe it frees the probe slot.

## Stale cache

`get_or_refresh(fetch)` returns the cached value while it is fresh. Past the
fresh TTL, one caller fetches; everyone else gets the stale value without
waiting. A failed fetch serves the stale value and starts a cooldown during
which nobody fetches. Once the stale window has passed, callers get an error.
`force_refresh(fetch)` fetches even when the value is fresh, for an unknown
key id, but at most once per cooldown. Every result says whether the value was
`Fresh`, `Stale` or `Fetched`, and `refresh_failed` is set when this call
fetched, failed, and got the cached value instead.

## Test vectors

`fixtures/resilience/breaker-v1.json` and `backoff-v1.json` pin the state
machine and the delay formulas. `tests/vectors.rs` replays them, and the
TypeScript client replays the same files.
