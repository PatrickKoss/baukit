# BaukitWorkerFailureRateHigh

## Meaning

More than 10% of runs of one `job_kind` ended with `outcome="failure"` over 10 minutes. Failure
means the job gave up: a permanent error, or the last attempt failed.

## Impact

Work in that job kind is lost or delayed until someone retries it. What that means for users
depends on the job: missed emails, unsynced data, stale exports.

## First checks

Outcomes per job kind:

```promql
sum by (job_kind, outcome) (baukit:worker_job_runs:rate5m{service="<service>", environment="<environment>"})
```

Recent failed jobs and their reason, from the product database:

```sql
SELECT id, job_type, failure_reason, left(last_error, 200), updated_at
FROM job_outbox WHERE status = 'failed' AND job_type = '<job_kind>'
ORDER BY updated_at DESC LIMIT 20;
```

Worker errors:

```logql
{service="<service>", environment="<environment>", level="error"} | json | line_format "{{.target}} {{.message}}"
```

## Fix

- A downstream API rejects the calls: fix credentials or payload, then requeue the failed rows
  through the worker's operations port (never the public listener). Collect the ids with the
  query above, up to 1000 per call:

  ```bash
  kubectl -n <namespace> port-forward deploy/<service>-worker 9090:<ops-port>
  curl -sS -X POST localhost:9090/admin/jobs/requeue \
    -H 'content-type: application/json' \
    -d '{"jobType": "<job_kind>", "ids": ["<id>", "<id>"]}'
  ```

  The answer lists the ids that went back to pending with a fresh attempt budget. Ids that are
  not failed, or whose dedupe key a newer active job holds, are left alone. A product that has
  not mounted `baukit_ops::jobs_admin::requeue_failed_jobs_router` on its ops listener gets a
  404; mount it rather than editing rows by hand.
- A code bug in the handler: roll back the worker image, then requeue.
- Bad payloads from one producer: fix the producer. Do not requeue rows that will fail again.

## Escalation

Product team. Page if the job kind sends user-visible messages or payments.
