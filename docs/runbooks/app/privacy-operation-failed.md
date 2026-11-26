# BaukitPrivacyOperationFailed

## Meaning

A job of kind `identity.account.delete` or `data-subject.*` reached `outcome="failure"` in the
last 15 minutes. The job runner gave up on it: a permanent error, the last attempt failed, or the
last lease expired. A database trigger then moved the matching operation to `failed`.

## Impact

A user asked for an erasure or an export and did not get it. Erasure and export requests carry
legal deadlines, so treat every firing as a ticket, even a single one.

## First checks

The failed operations and their codes:

```sql
SELECT id, kind, status, error_code, completed_at
FROM data_subject_operations WHERE status = 'failed'
ORDER BY completed_at DESC LIMIT 20;

SELECT id, state, created_at
FROM erasure_operations WHERE state = 'failed'
ORDER BY created_at DESC LIMIT 20;
```

The job rows behind them:

```sql
SELECT job_type, failure_reason, left(last_error, 200), updated_at
FROM job_outbox WHERE status = 'failed'
  AND (job_type = 'identity.account.delete' OR job_type LIKE 'data-subject.%')
ORDER BY updated_at DESC LIMIT 20;
```

Whether the identity provider was down at the time:

```promql
max by (destination) (egress_circuit_state{service="<service>", destination=~"keycloak-admin|idp"})
```

## Fix

- The IdP or a downstream store was down: wait until it recovers, then ask the user to submit
  the request again. Failed operations stay rerunnable with the same idempotency key.
- `export_too_large`: the export hit the part limit. Raise the limit in the product config or
  split the category, then rerun.
- A handler bug: roll back the worker image, then rerun.

Record each failed request and its outcome in the privacy log, whatever the fix.
