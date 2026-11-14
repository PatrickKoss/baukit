# BaukitWorkerQueueOldestAgeHigh

## Meaning

`worker_queue_oldest_age_seconds` for a `queue` has been above 300s for 10 minutes. The oldest job
that is ready to run has waited more than five minutes.

## Impact

Background work lags. Users see delayed notifications, syncs and exports.

## First checks

Age per queue and worker throughput:

```promql
worker_queue_oldest_age_seconds{service="<service>", environment="<environment>"}
```

```promql
sum by (outcome) (baukit:worker_job_runs:rate5m{service="<service>", environment="<environment>"})
```

Are workers running at all?

```sh
kubectl -n <namespace> get pods -l baukit.dev/process=worker,app.kubernetes.io/part-of=<product>
```

Backlog by job type:

```sql
SELECT job_type, status, count(*), min(run_after)
FROM job_outbox WHERE status IN ('pending', 'running')
GROUP BY 1, 2 ORDER BY 3 DESC;
```

Running jobs with expired locks point at a worker that died mid-job:

```sql
SELECT id, job_type, locked_by, locked_until FROM job_outbox
WHERE status = 'running' AND locked_until < now();
```

## Fix

- No worker pods, or they crash: fix the crash and scale the worker back up.
- Throughput is fine but the backlog grew: raise worker replicas or concurrency in the overlay.
- Retries crowd the queue: see `BaukitWorkerRetryStorm`.

## Escalation

Product team. Page if the age keeps growing after scaling, or passes an hour.
