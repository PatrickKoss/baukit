# BaukitWorkerRetryStorm

## Meaning

Jobs of one `job_kind` are retried more than once per second, sustained for 10 minutes. The jobs
fail with a retryable error and go back to the queue.

## Impact

Retries burn worker capacity and hammer the dependency that is failing. Other job kinds wait
longer. If attempts run out, the retries turn into failures.

## First checks

Retry rate next to success rate:

```promql
sum by (job_kind, outcome) (baukit:worker_job_runs:rate5m{service="<service>", environment="<environment>"})
```

Queue age, which rises when retries crowd out new work:

```promql
worker_queue_oldest_age_seconds{service="<service>", environment="<environment>"}
```

Pending jobs close to their attempt limit:

```sql
SELECT job_type, attempts, max_attempts, count(*)
FROM job_outbox WHERE status = 'pending' AND attempts > 0
GROUP BY 1, 2, 3 ORDER BY 4 DESC;
```

The retryable error:

```logql
topk(5, sum by (message) (count_over_time({service="<service>", environment="<environment>", level=~"warn|error"} | json [10m])))
```

## Fix

- The dependency is down: wait for it, or pause the job kind if retries make its recovery harder.
- A timeout that is too short: raise it so a slow but healthy call does not count as a failure.
- A permanent error classed as retryable: return a permanent error from the handler so the job
  fails once instead of looping.

## Escalation

Product team. Escalate to the owner of the failing dependency once identified.
