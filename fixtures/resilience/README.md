# Resilience vectors

Shared test vectors for the circuit breaker and backoff in
`rust/crates/baukit-resilience`. Any implementation, in any language, must
reproduce every expected value. All times are whole milliseconds and all
arithmetic below uses IEEE 754 doubles, the same in Rust and JavaScript.

## backoff-v1.json

`cases` is a list of independent calls:

| Field | Meaning |
|---|---|
| `strategy` | `full_jitter` or `decorrelated_jitter` |
| `initial_ms`, `max_ms` | Backoff bounds. If `initial_ms > max_ms`, use `max_ms` as the initial value. |
| `attempt` | Zero-based retry number |
| `fraction` | Jitter value in `[0, 1)` |
| `expected_delay_ms` | The delay to wait |

With `initial = min(initial_ms, max_ms)`:

- `full_jitter`: `floor(min(max_ms, initial * 2^attempt) * fraction)`
- `decorrelated_jitter`: `initial + floor((min(max_ms, initial * 3^attempt) - initial) * fraction)`

Products saturate at `2^32 - 1` before the `min`.

## breaker-v1.json

`policy` holds the policy every scenario uses:

| Field | Meaning |
|---|---|
| `window` | Recent calls the failure rate covers |
| `min_calls` | Calls the window needs before the rate can open the breaker |
| `failure_rate_percent` | Opens when `failures * 100 >= failure_rate_percent * calls` |
| `consecutive_timeouts` | Timeouts in a row that open the breaker |
| `open_base_ms`, `open_max_ms` | Open time for the first opening, and the cap for every open time |
| `jitter_percent` | Share taken off each open time |
| `probe_timeout_ms` | A half-open probe silent for this long is replaced |

Each scenario starts a fresh breaker in `closed` with time `0` and applies
`events` in order. Every event has `at_ms` (absolute time) and `op`:

- `admit`: ask whether a call may go ahead. Check `expect_admission`
  (`allowed`, `probe` or `rejected`) and, for `rejected`,
  `expect_retry_after_ms`. An admitted call returns a permit; when the event
  has `permit`, store it under that label.
- `record`: report an `outcome` (`success`, `failure`, `timeout`, `ignored`,
  or `hold_for` with `hold_for_ms`) for the call admitted with the permit
  labelled `permit`, using jitter `fraction`.

After every event the state must equal `expect_state` (`closed`, `half_open`
or `open`). An open breaker stays `open` until an `admit` at or after its end
moves it to `half_open`.

Permits: the breaker keeps a generation counter, starting at 0. It goes up by
one on every phase change (including half-open to half-open after an ignored
probe) and on every new probe. A permit carries its kind (`allowed` or
`probe`) and the generation at the moment it was issued, after any change the
admission itself caused. An outcome counts only if its permit's generation is
the current one and its kind matches the phase: `allowed` in closed, `probe`
in half-open. Any other outcome is dropped, except `hold_for` while open, which
applies with any permit.

Rules:

- Closed: `success`, `failure` and `timeout` enter the window (the last
  `window` calls); `failure` and `timeout` count as failures. `timeout` adds one
  to the consecutive-timeout count, `success` and `failure` reset it. `ignored`
  changes nothing. After each entry the breaker opens if the timeout count
  reaches `consecutive_timeouts`, or the window holds at least `min_calls`
  calls and the failure rate is reached. Opening clears the window and the count.
- Opening number `n` since the breaker last closed lasts
  `open = min(open_max_ms, open_base_ms * 2^(n - 1))` minus
  `floor(open * ((jitter_percent / 100) * fraction))`.
- `hold_for` opens the breaker for `min(hold_for_ms, open_max_ms)` and does not
  change `n`. While open it only ever extends the end, and other outcomes are
  dropped.
- Open: `admit` before the end is `rejected` with the time left; at or after
  the end it moves to `half_open` and returns `probe`.
- Half-open: `admit` is `rejected` with the time left on the probe lease
  (`probe_timeout_ms` from the probe's admission), or `probe` when no probe is
  out or its lease has run out. `success` closes the breaker and resets `n`;
  `failure` or `timeout` opens it with `n + 1`; `ignored` frees the probe slot
  and spends the permit, so the next `admit` returns a new probe.
