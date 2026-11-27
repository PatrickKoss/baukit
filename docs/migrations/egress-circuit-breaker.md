# Egress destinations, circuit breaker and in-flight cap

`baukit-egress` 0.32.0 names every client, puts a circuit breaker and an in-flight cap in front of
every `(scheme, host, port)`, and caps outbound timeouts with the inbound request's deadline. Both
guards are on by default. [resilience-contract.md](../platform/resilience-contract.md) holds the
policies and the list of built-in destination names.

## Name every client

`GuardedClient::new` and `GuardedClient::with_resolver` take a destination name first:

| 0.31.x | 0.32.0 |
| --- | --- |
| `GuardedClient::new(options)` | `GuardedClient::new("payments", options)` |
| `GuardedClient::with_resolver(resolver, options)` | `GuardedClient::with_resolver("payments", resolver, options)` |

The name is the `destination` label on `egress_requests_total`, `egress_request_duration_seconds`,
`egress_circuit_state` and `egress_circuit_transitions_total`. Use one short, lowercase, static name per
provider. Never put a host, tenant, user or URL in it. Do not reuse the names baukit crates already use:
`oidc`, `idp`, `hub`, `suite-peer`, `expo`, `keycloak-admin` and `mcp-introspection`. Generated
products get `docs/outbound-providers.md` with the same rules.

## The breaker and the cap

Each `(scheme, host, port)` gets a breaker with `BreakerPolicy::default()`: it opens at 50% failures over
the last 20 calls (at least 10) or after 5 consecutive timeouts, stays open 30 s, and doubles on each
failed probe up to 5 min. A `429`, or a `5xx` with `Retry-After`, holds the circuit open for exactly
that delay. Each `(scheme, host, port)` also allows 32 requests in flight.

Two new `EgressError` variants send nothing:

| Variant | `code()` | `retry_class()` |
| --- | --- | --- |
| `CircuitOpen { retry_after }` | `circuit_open` | `RetryAfter(retry_after)` |
| `Saturated` | `saturated` | `RetryAfter(1 s)` |

A `match` on `EgressError` without a wildcard arm must handle both. Tune or turn the guards off on the
options:

```rust
let options = EgressOptions::default()
    .with_circuit_breaker(BreakerPolicy { open_base: Duration::from_secs(10), ..BreakerPolicy::default() })
    .with_max_in_flight(8)?;
let one_shot = EgressOptions::default().without_circuit_breaker();
```

`with_max_in_flight(0)` fails with `EgressOptionsError::ZeroMaxInFlight`. Keep the breaker on in
services. `without_circuit_breaker` is for tests and one-shot tools.

## Deadlines

Inside an inbound request, the client's request timeout is capped by the time left on the request
deadline that `baukit-http` sets. With less than 50 ms left the call fails at once with `EgressError::Timeout`. A
timeout counts against the breaker only when the time left was at least half the client's own request
timeout, so a nearly expired inbound request does not open the circuit.

## Job handlers

An open circuit or a full cap is not the job's fault, so a job handler should defer the job instead of
spending an attempt. Turn on the `egress` feature of `baukit-jobs` and map the error:

```toml
baukit-jobs = { version = "=0.32.0", features = ["egress"] }
```

```rust
use baukit_jobs::{JobError, egress_job_error};

let response = payments
    .execute(request)
    .await
    .map_err(|error| egress_job_error(&error))?;
```

`egress_job_error` maps `CircuitOpen` to `JobError::deferred` with the breaker's delay and `Saturated`
to a 1 s deferral. Every other error keeps its retry class: a provider delay becomes
`JobError::retryable_after`, other retryable failures `JobError::retryable`, and blocked addresses,
revoked credentials and permanent statuses `JobError::permanent`.
[job-deferrals-and-concurrency-keys.md](job-deferrals-and-concurrency-keys.md) covers deferrals and
`max_deferrals`.

Inside an HTTP handler, answer `ApiError::unavailable(retry_after)` with the error's `retry_after`
instead.

## Delete product retry loops

`GuardedClient` never retries, and the breaker now counts every call. A product loop that retries a
`GuardedClient` call multiplies load on a failing provider and opens the circuit faster. Delete it. In
a job, the runner's attempts and deferrals are the retry layer. In a request, return `503` and let the
client retry.
