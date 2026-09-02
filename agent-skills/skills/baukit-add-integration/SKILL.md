---
name: baukit-add-integration
description: Design, implement, or review a reliable product-owned external integration in a Baukit product. Use for outbound provider APIs, OAuth-backed connections, webhooks, import/export workers, notification delivery, provider fakes, retry behavior, or integration observability where durable execution should compose with baukit-jobs.
---

# Add a reliable integration

Keep provider models, OAuth grants and recovery, webhook schemas and signature rules, cursors, normalized domain entities, and user-facing recovery flows in the product. Use Baukit's runtime and durable-job primitives; do not generate another generic queue or a connector framework.

## Classify the boundary first

Before implementing, write down:

1. One timeout for every outbound call, including token exchange and pagination.
2. A closed product error classification: success, rate-limited, unavailable, timed-out, revoked/terminal-auth, invalid request or payload, and other permanent failure.
3. Which failures are retryable, any provider `Retry-After` mapping, the attempt cap, idempotency key, and the queryable terminal result.
4. Which accepted ingress and local domain writes must commit atomically with durable work.
5. Stable machine error codes and the product-local localized copy/recovery action for each user-actionable outcome.

Never log credentials, authorization headers, signature secrets, provider payloads, or raw exception text.

## Build the client connection flow

Use `@baukit/integrations-client` to reduce server health and client events into connection states and
available actions. Pass only stable snake_case diagnostic codes. Never render the diagnostic field or
copy raw provider errors into it.

Coordinate OAuth through `OAuthSessionCoordinator`. Inject product storage, nonce creation, timers,
clock, popup or native runners, and any same-tab redirect handler. Configure an exact return origin
and path allowlist. The product still owns provider scopes, authorization parameters, persistence,
identity, recovery policy, and localized copy. Build `ProviderRegistry` entries from those values;
do not add provider-specific branches to the package.

## Use `baukit-jobs` for durable work

Define static job types in a product `JobHandler`. Enqueue with `NewJob`; use `PostgresJobStore::enqueue_in_transaction` when the domain write and outbox row share PostgreSQL. Use `JobError::retryable` for runner backoff, `JobError::retryable_after` for a provider-directed delay, and `JobError::permanent` for terminal classification. The runner still enforces `max_attempts`.

Keep the existing lifecycle: `pending`, `running`, `succeeded`, `failed`, and `cancelled`. Do not add `dead_letter`. Query terminal `failed` jobs through `Job.failure_reason`, which distinguishes `JobFailureReason::Permanent` from `JobFailureReason::AttemptsExhausted`; keep `last_error` diagnostic, bounded, and secret-free.

When a handler writes a PostgreSQL result, finish with `PostgresJobStore::complete_in_transaction` in the same transaction, using `JobCancellation::worker_id()` as the lease owner. After commit, call `JobCancellation::mark_completed_in_transaction()` and return success immediately so the runner does not attempt a second completion. External side effects still require a destination idempotency key because a lease can expire after the provider accepted a request.

## Make accepted ingress durable

Verify webhook authenticity and replay bounds against the exact raw request before persistence. Do not enqueue or parse trusted domain data before verification.

- With one PostgreSQL database, insert the deduplicated inbox record and enqueue the outbox job in one transaction. Return acceptance only after commit.
- When enqueue uses another system or can fail independently, persist the accepted inbox first as the durable source. Run a bounded drainer/reconciler that finds accepted-but-not-enqueued rows, retries safely, and exposes terminal/age state. Never acknowledge a delivery that exists only in memory.

For batches, enqueue one external consequence per job or record each item's outcome independently. One malformed or rejected item must not erase successful siblings or make the whole batch look successful.

## Build deterministic failure fakes

Provide controllable fakes for `healthy`, `rate_limited`, `unavailable`, `timeout`, `revoked`, and `exhausted`. Assert timeout enforcement, exact classification, provider-directed retry scheduling, attempt exhaustion, idempotent replay, process restart/lease recovery, webhook rejection before persistence, drainer reconciliation, and per-item isolation. Use fixed clocks and explicit barriers or notifications instead of timing races.

## Keep errors and telemetry bounded

Expose stable snake_case machine codes to clients. Resolve those codes plus safe structured details to localized copy and recovery actions in product UI; never render provider exception strings.

Register metric names before emitting them. Label only with build-time-bounded values such as static provider, job, operation, or outcome enums. Never label with user IDs, connection IDs, URLs, tokens, payload fields, or error text. Reuse Baukit worker metrics rather than recording the same run twice.

Client delivery queues also need a size bound, attempt bound, idempotency/replay rule, and an inspectable terminal state. Do not report “synced” or “delivered” while retryable or terminal items remain hidden.

## Verify the slice

Run unit tests for classification and fakes, PostgreSQL integration tests including ignored Docker tests, worker telemetry conformance, migration checks, and a generated worker fixture when shared APIs or templates changed. Demonstrate one real product worker slice on `baukit-jobs` before proposing more platform abstractions; Fitness-style integration workers should migrate onto these primitives rather than preserve a second queue.
