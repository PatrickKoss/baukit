# BaukitHighErrorBudgetBurnSlow

## Meaning

The 5xx ratio of one service burns its 99.9% error budget more than 6 times faster than allowed in
both the 30m and the 6h window, for 15 minutes. A 30-day budget lasts about five days at this rate.
This is a slow leak, not an outage.

## Impact

A small share of requests fail all the time. Users notice retries and intermittent errors. Left
alone it uses up the budget for the month.

## First checks

The burn rate over the long windows:

```promql
baukit:http_requests_5xx:error_budget_burn_rate6h{service="<service>", environment="<environment>"}
```

Routes that contribute most, over 6h so short spikes do not hide the steady offender:

```promql
topk(5, sum by (route, status) (increase(http_requests_total{service="<service>", environment="<environment>", status=~"5.."}[6h])))
```

When it started, relative to releases:

```sh
kubectl -n <namespace> get replicasets -l baukit.dev/process=api,app.kubernetes.io/part-of=<product> \
  --sort-by=.metadata.creationTimestamp
```

Recurring error messages:

```logql
topk(10, sum by (message) (count_over_time({service="<service>", environment="<environment>", level="error"} | json [6h])))
```

## Fix

- One route with a steady error: fix the handler or the input validation behind it. A 5xx for bad
  client input should be a 4xx.
- Errors that follow load: compare with `BaukitDatabasePoolSaturated` and the HPA
  (`kubectl -n <namespace> get hpa`) and scale or raise the pool size.
- Errors that started with a release: roll back as in the fast-burn runbook.

## Escalation

Open a ticket with the product team during working hours. Treat it as urgent if the fast-burn
alert also fires or the budget for the month is already below 25%.
