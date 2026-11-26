# BaukitErrorBudgetBurnTicket

## Meaning

The 5xx ratio of one service has burned its 99.9% error budget faster than 1x in both the 1d and the
3d window, for an hour, with at least 100 requests in the 3d window. At this rate the 30-day budget is
gone before the month ends. The fast and slow burn alerts miss this: the leak is too small to page.

## Impact

A steady trickle of failed requests. Nobody notices one of them, but they add up to a missed SLO.

## First checks

Burn over the ticket windows, and how much of the month's budget is left (the "Availability error
budget remaining" panel on the service overview dashboard shows the same):

```promql
baukit:http_requests_5xx:error_budget_burn_rate3d{service="<service>", environment="<environment>"}
```

```promql
1 - (sum(increase(http_requests_total{service="<service>", environment="<environment>", status=~"5.."}[30d])) / sum(increase(http_requests_total{service="<service>", environment="<environment>"}[30d]))) / 0.001
```

The routes and codes behind it, over 3d:

```promql
topk(10, sum by (route, status) (increase(http_requests_total{service="<service>", environment="<environment>", status=~"5.."}[3d])))
```

## Fix

- One route dominates: fix that handler. A 5xx for bad client input should be a 4xx.
- 503s spread over every route, bunched around deploys or node drains: the service sheds or drops
  requests while pods restart. Check the rollout and the PDB (`kubectl -n <namespace> get pdb`).
- 503s with `Retry-After` at peak times: load shedding works as designed but capacity is short. Scale
  the API or raise its concurrency limit.

## Escalation

Ticket for the product team, handled within the week. No page.
