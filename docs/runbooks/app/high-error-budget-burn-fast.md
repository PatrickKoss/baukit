# BaukitHighErrorBudgetBurnFast

## Meaning

The 5xx ratio of one service burns its 99.9% error budget more than 14.4 times faster than allowed
in both the 5m and the 1h window. At this rate a 30-day budget is gone in about two days. The alert
fires after 2m and carries the `product`, `service` and `environment` labels.

## Impact

Users of `service` get server errors right now. Every hour at this rate spends about 2% of the
monthly budget.

## First checks

Which routes fail, and how badly:

```promql
topk(10, baukit:http_requests_5xx:rate5m{service="<service>", environment="<environment>"})
```

```promql
sum by (route, status) (rate(http_requests_total{service="<service>", environment="<environment>", status=~"5.."}[5m]))
```

Did a rollout just happen?

```sh
kubectl -n <namespace> rollout history deployment -l baukit.dev/process=api,app.kubernetes.io/part-of=<product>
kubectl -n <namespace> get pods -l baukit.dev/process=api,app.kubernetes.io/part-of=<product> -o wide
```

Error logs from the API, with the failing target module:

```logql
sum by (target) (count_over_time({service="<service>", environment="<environment>", level="error"} | json [5m]))
```

```logql
{service="<service>", environment="<environment>", level="error"} | json | line_format "{{.target}} {{.message}}"
```

Check the dependencies the errors name. `BaukitDatabaseAcquireTimeouts` and
`BaukitRateLimitStoreErrors` often fire together with this alert.

## Fix

- A bad release: `kubectl -n <namespace> rollout undo deployment/<release>-api`, or revert the image
  tag in the product's GitOps overlay so Flux does not roll it forward again.
- A failing dependency (database, Redis, an upstream API): follow that alert's runbook.
- One route only: disable the feature behind it or shed its traffic at the ingress until fixed.

## Escalation

Page the product owner of `product` if errors continue 15 minutes after a rollback, or if no
release explains them. Escalate to the platform on-call when several products burn at the same
time, since that points at shared infrastructure.
