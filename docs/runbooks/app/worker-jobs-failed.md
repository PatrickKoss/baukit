# BaukitWorkerJobsFailed

## Meaning

`worker_queue_depth{status="failed"}` grew in one queue within the last 15 minutes. A job ends as
`failed` when it returns a permanent error or uses its last attempt. The runner never picks a failed
row up again. Suite peer deliveries, erasure and data-subject jobs all run through this outbox, so a
failed row is a delivery that did not happen.

## Impact

The work in those rows is lost until someone requeues it. For suite deliveries, a peer product
misses events or revocations.

## First checks

Failed rows per queue:

```promql
max by (queue) (worker_queue_depth{service="<service>", environment="<environment>", status="failed"})
```

Which job kinds fail, and why:

```sql
SELECT job_type, failure_reason, count(*), max(updated_at)
FROM job_outbox WHERE status = 'failed' AND updated_at > now() - interval '1 hour'
GROUP BY job_type, failure_reason ORDER BY count(*) DESC;
```

Open circuit breakers or failing destinations behind the jobs:

```promql
max by (destination) (egress_circuit_state{service="<service>", environment="<environment>"})
```

## Fix

- A destination was down longer than the attempts last: once it answers again, requeue the rows
  by setting `status = 'pending', attempts = 0, failure_reason = NULL, last_error = NULL` on them.
- A permanent error such as a revoked peer or a rejected payload: fix the cause first. Requeued
  rows fail again otherwise.
- A handler bug: roll back the worker image, then requeue.

## Escalation

Product team. Page if the failed jobs carry revocations or erasure requests, which have deadlines.
