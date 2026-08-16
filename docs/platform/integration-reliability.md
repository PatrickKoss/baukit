# Integration reliability

**Status:** Product-facing implementation recipe; no generic connector model.
**Applies to:** Outbound provider calls, OAuth-backed connections, webhooks, imports, notifications, and client delivery queues.
**Related:** [telemetry specification](./telemetry-spec.md) and [offline readiness](./offline-readiness-contract.md).

Baukit supplies durable execution and operational contracts. Provider resources, OAuth models and recovery, webhook schemas and verification rules, cursor formats, credentials, normalized entities, and localized product UX remain product-local. A collection of providers in one domain is not evidence for a shared connector framework.

## 1. Outbound boundary

Every outbound operation must set a finite timeout at the HTTP client or operation boundary. Classify its result explicitly into a closed product vocabulary such as:

| Class | Typical handling |
|---|---|
| healthy | Commit the result and mark the job successful. |
| rate-limited | Retry using a valid provider delay when present. |
| unavailable | Retry with bounded runner backoff. |
| timed-out | Treat the outcome as unknown; reconcile or replay idempotently. |
| revoked or terminal auth | Stop retrying and surface a stable reconnect/action code. |
| invalid request or payload | Permanent failure; repair product input or mapping. |

Do not infer success from a completed HTTP exchange. Validate the provider response and the product's success predicate. Never include credentials, authorization headers, webhook secrets, provider bodies, or raw exception text in logs or durable error fields.

## 2. Durable retries with `baukit-jobs`

Use `NewJob` with an explicit `max_attempts`, a static `job_type`, and an idempotency key representing the intended effect. Product `JobHandler` implementations return:

```rust
use std::time::Duration;

use baukit_jobs::JobError;

fn classify_rate_limit(retry_after: Duration) -> JobError {
    JobError::retryable_after("provider rate limit", retry_after)
}

fn classify_unavailable() -> JobError {
    JobError::retryable("provider unavailable")
}

fn classify_revoked() -> JobError {
    JobError::permanent("provider authorization revoked")
}
```

`WorkerRunner` honors `JobError::retryable_after` instead of its exponential delay, but the persisted `max_attempts` cap remains authoritative. Without a hint, it uses `WorkerConfig.retry_initial` and `retry_max`.

Terminal work stays in the existing `failed` lifecycle state. `Job.failure_reason` is `JobFailureReason::Permanent` for a non-retryable handler result and `JobFailureReason::AttemptsExhausted` when retryable work or an expired final lease consumes the cap. `last_error` is bounded diagnostic text, not a machine classification or localized message. Do not introduce `dead_letter` as another state.

Query pending and failed work in product operations surfaces so retry schedules, attempt counts, last safe diagnostics, and terminal reasons are inspectable. Cancellation, timeout, and lease recovery must not bypass the attempt cap.

## 3. Atomic writes and accepted webhooks

When a domain mutation and its durable consequence use one PostgreSQL database, begin a product transaction, write the domain/inbox record, call `PostgresJobStore::enqueue_in_transaction`, and commit once. When a handler writes its result in PostgreSQL, write the result and call `PostgresJobStore::complete_in_transaction` last in the same transaction.

External side effects still require a provider idempotency key. A worker can lose its lease after the provider accepted the call but before local completion committed.

For webhooks:

1. Read the bounded raw body needed by the signature scheme.
2. Verify signature, timestamp/replay window, and expected provider/tenant context before persistence.
3. Deduplicate on a stable delivery identifier.
4. In one PostgreSQL transaction, persist the verified inbox row and enqueue its outbox job.
5. Return an accepted response only after the transaction commits.

If enqueue uses another system or can fail independently, the accepted inbox must be the durable source. A bounded drainer/reconciler claims accepted-but-not-enqueued rows, retries transfer idempotently, and exposes age, attempts, and terminal state. An in-memory handoff after returning success can silently lose accepted delivery and is forbidden.

## 4. Batches and client queues

Prefer one external consequence per durable job. When a provider requires batching, record an outcome for each item and retry only retryable items; a single malformed or revoked item must not discard successful siblings or cause the whole batch to appear successful.

Client delivery queues need both size and attempt bounds. Persist an idempotency key, next attempt, attempt count, and inspectable terminal code. Define overflow and discard behavior explicitly. “Delivered” or “synced” is false while hidden pending or terminal items remain.

## 5. Stable failures and localized recovery

Map provider/transport failures to stable snake_case machine codes at the product boundary, for example `provider_rate_limited`, `provider_unavailable`, `provider_revoked`, or `delivery_exhausted`. APIs return those codes with safe structured details. Web and mobile map `code + details` to localized copy and recovery actions; they never display `last_error` or an exception string.

Provider connection state, reconnect flows, OAuth consent, scopes, and recovery copy remain product-local even when their failure classes follow this recipe.

## 6. Deterministic fakes and acceptance checks

Provide deterministic adapters for `healthy`, `rate_limited`, `unavailable`, `timeout`, `revoked`, and `exhausted`. Prefer fixed clocks, explicit barriers, and manually released futures over sleep-sensitive races.

Acceptance must prove:

- every outbound path times out and maps to the expected stable class;
- provider retry hints set `run_after`, while `max_attempts` still produces `failed` with `attempts_exhausted`;
- permanent/revoked work produces `failed` with `permanent` and does not retry;
- destination idempotency makes timeout, crash, lease expiry, and replay safe;
- webhook verification fails before persistence, and an accepted delivery survives restart before enqueue;
- the inbox/outbox transaction rolls back together, or the drainer reconciles an independently persisted inbox;
- batch failures are isolated per item;
- client queue overflow, retries, and terminal items are bounded and visible; and
- no fake, log, metric, or public error leaks a credential or dynamic provider message.

Run unit tests plus PostgreSQL integration tests with Docker-gated ignored tests enabled. Exercise worker telemetry conformance and generated worker fixtures whenever shared APIs or templates change.

## 7. Telemetry bounds and adoption

Reuse Baukit's `worker_job_runs_total`, `worker_job_duration_seconds`, and `worker_queue_oldest_age_seconds`. Additional product metrics must be registered and use bounded code-defined labels such as provider, operation, job, and outcome enums. Never use secrets, user/connection IDs, URLs, payload fields, exception text, request IDs, or other dynamic strings as labels.

Fitness-style worker slices should migrate onto `baukit-jobs` or a thin product adapter around it. Record real missing primitives during that adoption before extending Baukit. Provider models, OAuth behavior, webhook payloads, recipe imports, food databases, and push semantics remain in their products.
