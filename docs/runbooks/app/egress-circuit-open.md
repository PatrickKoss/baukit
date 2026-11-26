# BaukitEgressCircuitOpen

## Meaning

The circuit breaker for one destination has reported open for 5 minutes. `egress_circuit_state` is the
worst state across the `(scheme, host, port)` breakers of the destination: `0` closed, `1` half-open,
`2` open. A breaker opens at 50% failures over the last 20 calls (at least 10), after 5 consecutive
timeouts, or for exactly the `Retry-After` a destination sent with `429` or `503`. It stays open 30 s,
doubling on each failed probe up to 5 minutes.

## Impact

Calls to the destination fail at once with `circuit_open` and a `Retry-After` until a probe succeeds.
Five minutes open means several probes failed in a row, or the destination asked for a long pause.

## First checks

Transitions, to tell a flapping breaker from one that never recovers:

```promql
sum by (to) (increase(egress_circuit_transitions_total{service="<service>", environment="<environment>", destination="<destination>"}[30m]))
```

What the probes got back:

```promql
sum by (outcome, status) (rate(egress_requests_total{service="<service>", environment="<environment>", destination="<destination>", outcome!="circuit_open"}[15m]))
```

A breaker only moves to half-open when a call arrives. With no traffic to the destination the gauge keeps
reporting open, so check the request rate before assuming the destination is still down.

## Fix

- Probes fail with 5xx, timeouts or connect errors: the destination is down. Wait for it, or fail over if
  the product has a second provider.
- Probes get `429` or `503` with `Retry-After`: the provider is rate limiting this service. Lower the call
  rate (batching, caching) or ask the provider for a higher quota.
- No calls at all: the gauge is stale. The next call sends a probe; nothing is broken.

## Escalation

Product team. Contact the provider when its status page shows no incident.
