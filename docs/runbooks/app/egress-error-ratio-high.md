# BaukitEgressErrorRatioHigh

## Meaning

More than 10% of the outbound calls from the service to one destination failed for 10 minutes, with at
least one call every 20 seconds. Failed means the outcome in `egress_requests_total` was `unavailable`
(connect, DNS or 5xx), `timeout`, `circuit_open` or `saturated`. `4xx`, `revoked` and `permanent`
outcomes are bugs or bad credentials on our side and do not count. The `destination` label is the
name the client was built with (`hub`, `oidc`, `expo`, a provider name), not a host.

## Impact

Features that depend on the destination degrade. Jobs that call it retry or defer, so their queue grows.
Inbound requests that call it synchronously answer `503` while the circuit is open.

## First checks

Outcomes and statuses over time:

```promql
sum by (outcome, status) (rate(egress_requests_total{service="<service>", environment="<environment>", destination="<destination>"}[5m]))
```

Is the breaker open, and since when?

```promql
max by (destination) (egress_circuit_state{service="<service>", environment="<environment>"})
```

Failed calls in the logs (`error.type` holds the error code, never the URL):

```logql
{service="<service>", environment="<environment>"} | json | message="egress request failed"
```

## Fix

- `unavailable` with 5xx or `timeout`: the destination is down or slow. Check its status page or, for
  `hub` and `suite-peer`, the other service's own alerts. The breaker already stops most calls; nothing
  to do here until the destination recovers.
- `unavailable` with `status="none"`: DNS or connect failures. Check egress network policy and DNS
  from the pod.
- `saturated`: more than 32 calls to one host were in flight. The destination is slow, or a caller fans
  out without a limit. Raise `with_max_in_flight` only after checking the second case.
- `circuit_open` alone: follow `BaukitEgressCircuitOpen`.

## Escalation

Product team. Contact the provider when its status page shows no incident.
