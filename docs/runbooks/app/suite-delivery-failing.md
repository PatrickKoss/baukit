# BaukitSuiteDeliveryFailing

## Meaning

More than 10% of the suite event deliveries from the service to its peers failed for 15 minutes, with
at least one delivery every 100 seconds. `suite_deliveries_total` counts one sample per delivery
attempt. Failed means outcome `retry` (network error or 5xx from the peer), `rejected` (the peer refused
the payload), `unauthorized` (the peer rejected the signature) or `disabled` (the link hit the failure
limit and only gets probes). `deferred` attempts waited for an open egress breaker and `revoked` links
were ended by the peer, so neither counts.

## Impact

Linked products see suite events late or not at all. Retries keep rows in the jobs outbox. A link that
reaches the failure limit is disabled and only probed, first after one hour.

## First checks

Outcomes over time:

```promql
sum by (outcome) (rate(suite_deliveries_total{service="<service>", environment="<environment>"}[5m]))
```

Is the peer reachable?

```promql
sum by (outcome, status) (rate(egress_requests_total{service="<service>", environment="<environment>", destination="suite-peer"}[5m]))
```

## Fix

- `retry`: the peer is down or slow. Check the peer product's own alerts. Deliveries resume on their
  own when it recovers.
- `unauthorized`: the link secret differs between the two products, often after a restore or a key
  rotation on one side. Relink the products.
- `rejected`: the peer does not accept the event type or payload version. Compare the payload catalogs
  of both releases.
- `disabled`: links reached the failure limit. Fix the cause above. The next delivered probe makes the
  link healthy again.

## Escalation

Product team, together with the team that owns the peer product.
