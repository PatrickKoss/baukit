# HLC rejects remote timestamps too far ahead

`HybridLogicalClock::observe` (Rust) and `observe` (TypeScript
`@baukit/sync-client`) now reject a remote timestamp more than 24 hours ahead
of local physical time with `RemoteTooFarAhead` / `remote_too_far_ahead`.
Before, one bad timestamp pushed every later timestamp forward with no limit.
Change the bound with `with_max_drift_ms` (Rust) or the `maxDriftMs` option of
`open` (TypeScript).

Servers should check client HLC values with `validate_client_timestamp` /
`validateClientTimestamp` and answer 422 on rejection.

Persisted state that fails to parse, or belongs to another device, now
restarts at physical time instead of zero. A clock that restarted at zero could
issue timestamps lower than ones it had already sent.

In browsers, `HybridLogicalClock` serializes ticks across tabs with
`navigator.locks` and re-reads storage under the lock. Without Web Locks it
behaves as before, so two tabs can still issue equal timestamps there.
