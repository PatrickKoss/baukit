# ApiHighErrorRatio

Alert rules in `deploy/observability/rules/` point their `runbook_url` at a file in this directory,
as a path relative to the repository root. Once the repository has a public or internal web URL,
replace the `docs/runbooks/` prefix in every rule with that URL, for example
`https://git.example.com/{{ context.app_name }}/blob/main/docs/runbooks/`. Add one runbook here per
new alert, with the same headings.

## Meaning

More than 5% of `{{ context.app_name }}` API requests returned a 5xx status over 5 minutes, sustained
for 10 minutes, in one `environment`.

## Impact

Users get server errors on a large share of requests. This is an outage for the affected routes.

## First checks

Failing routes and status codes:

```promql
sum by (route, status) (rate(http_requests_total{product="{{ context.app_name }}", environment="<environment>", status=~"5.."}[5m]))
```

API pods and recent rollouts:

```sh
kubectl -n <namespace> get pods -l baukit.dev/process=api,app.kubernetes.io/part-of={{ context.app_name }}
kubectl -n <namespace> rollout history deployment/<release>-api
```

Error logs:

```logql
{service="{{ context.app_name }}-api", environment="<environment>", level="error"} | json
```

Database pool timeouts, a frequent cause:

```promql
increase(db_pool_acquire_timeouts_total{product="{{ context.app_name }}", environment="<environment>"}[10m])
```

## Fix

- The errors started with a release: `kubectl -n <namespace> rollout undo deployment/<release>-api`,
  then revert the image tag in the deployment overlay.
- A dependency fails (database, Redis, an upstream API): restore it, then confirm the ratio drops.
- One route only: disable the feature behind it until the handler is fixed.

## Escalation

Page the {{ context.app_name }} owner if the ratio stays above 5% for 15 minutes after a rollback.
