# BaukitSuiteEmissionSkipped

## Meaning

The service skipped at least one live suite event in the last 15 minutes instead of writing it to the
jobs outbox. `suite_emission_skipped_total` counts these by `reason`: `invalid` (the payload failed the
catalog), `timeout` (the outbox write timed out) or `storage` (the outbox write failed). The business
write that produced the event still committed.

## Impact

Linked peers never receive the skipped events. Nothing retries them. A replay after relinking sends
the history again.

## First checks

Skips by reason:

```promql
sum by (reason) (increase(suite_emission_skipped_total{service="<service>", environment="<environment>"}[1h]))
```

The skipped event types are in the logs:

```logql
{service="<service>", environment="<environment>"} | json | message="Skipping suite emission"
```

## Fix

- `invalid`: the product emits a payload its own catalog rejects. This is a product bug. Fix the
  emitter or the catalog and release.
- `timeout` or `storage`: the database is slow or failing. Follow the database pool and acquire
  alerts for the same service.

## Escalation

Product team.
