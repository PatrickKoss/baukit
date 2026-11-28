# Suite metrics have fixed names

Before 0.33.0, `baukit-suite` put the product's app id in front of its metric names, so the delivery
counter of a product with app id `fitness` was `fitness_suite_deliveries_total`. Shared alert rules
cannot match a name that changes per product, so no alert covered suite delivery.

## What changed

| Before | Since 0.33.0 |
|---|---|
| `{app_id}_suite_deliveries_total` | `suite_deliveries_total` |
| `{app_id}_suite_events_ingested_total` | `suite_events_ingested_total` |
| `{app_id}_suite_emission_skipped_total` | `suite_emission_skipped_total` |

- Labels are unchanged. The product comes from the scrape labels (`service`, `environment`,
  `namespace`), as for every other baukit metric.
- `SuiteServiceConfig` and `SuiteContext` no longer have `metric_prefix`, and
  `PostgresSuiteEventOutbox::new` drops its last argument.
- `suite_deliveries_total{outcome="deferred"}` is now registered at zero on startup, like the other
  outcomes.
- New alerts in `deploy/observability/alerts/baukit.rules.yml` use the fixed names:
  `BaukitSuiteDeliveryFailing` and `BaukitSuiteEmissionSkipped`.

## What to do

1. Remove `metric_prefix` where you build `SuiteServiceConfig`, and the last argument of
   `PostgresSuiteEventOutbox::new`. The compiler finds both.
2. Rename the metrics in your own dashboards, recording rules and alerts. `git grep _suite_` finds
   them. History under the old names stays under the old names.
3. Add the new names to your `baukit check` metric list if your product keeps one.
