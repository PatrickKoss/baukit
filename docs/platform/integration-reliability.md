# Integration reliability

**Status:** Product-facing implementation recipe; the generic connector port is `baukit-integrations`.
**Applies to:** Outbound provider calls, OAuth-backed connections, webhooks, imports, notifications, and client delivery queues.
**Related:** [telemetry specification](./telemetry-spec.md), [offline readiness](./offline-readiness-contract.md), and [resource budgets](./resource-budgets-contract.md).

Baukit supplies durable execution and operational contracts. Provider resources, OAuth models and recovery, webhook schemas and verification rules, cursor formats, credentials, normalized entities, and localized product UX remain product-local. A collection of providers in one domain is evidence for a shared connector *contract* but not for a shared connector *framework*. `baukit-integrations` owns the port shape, the retry vocabulary, and the scripted fake; the product-local list above stays product-local. See [ADR 0002](../adr/0002-integration-connector-contract.md) for the reasoning and [the crate README](../../rust/crates/baukit-integrations/README.md) for the API.

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

Keeping credentials out of logs is easier when they are not lying around as plain `String` values in the first place. Store provider tokens, API keys, and webhook secrets through `baukit-credential-vault`. Its `CredentialSecrets` zeroizes plaintext on drop and implements neither `Debug` nor `Display`, so a stray `{:?}` cannot print one, and `CredentialCipher` encrypts each field under a versioned AES-256-GCM keyring bound to the credential scope and the field name. The keyring comes from `<APP>__CREDENTIAL_VAULT__KEYRING` and rotates without a data migration. The storage adapter stays product-local behind the crate's `CredentialVault` port, because the table shape and the ownership join are product decisions.

## 2. Credential probes

Credential validation is separate from paged import work. A product adapter
implements `baukit_integrations::CredentialProbe` and maps its provider to this
closed set:

| Probe result | Connection health | Retry |
|---|---|---|
| valid credential and opaque account ID | `healthy` | none |
| revoked credential | `needs_reconnect` | no |
| missing required scope | `needs_reconnect` | no |
| rate limited | `degraded` | provider `Retry-After`, when valid |
| timeout | `degraded` | product backoff |
| unavailable | `degraded` | product backoff |
| invalid or oversized data | `failed` | no |

The adapter owns the endpoint, authorization header, required scopes, accepted
statuses, response parser, and product account model. Baukit carries the account
ID without interpreting it. Do not log it or use it as a metric label.

Set a finite request timeout. Read no more than
`MAX_CREDENTIAL_PROBE_RESPONSE_BYTES`, including chunked responses whose
`Content-Length` is absent or false. Map a body over the limit to `InvalidData`
and discard it. Parse `Retry-After` with `baukit_http` and keep the returned
duration on `CredentialProbeError::RateLimited`. Public errors and logs may use
only `CredentialProbeError::code()`, never the credential, provider response,
request URL, or parser error.

Run `baukit_test::check_credential_probe_conformance` for every adapter. The
product supplies raw success, missing-scope, and malformed responses to the
scripted HTTP server. This is where provider-specific scope headers and JSON
remain. The shared runner checks the outcomes without a provider switch.

Migration is additive. Existing `IntegrationConnector` implementations and
import jobs do not change. Replace a product token-check port only after its
adapter passes the shared suite, then retain existing API codes by mapping the
new error code in the product service.

## 3. Durable retries with `baukit-jobs`

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

## 4. Accepted deliveries and PostgreSQL inboxes

When a domain mutation and its durable consequence use one PostgreSQL database, begin a product transaction, write the domain/inbox record, call `PostgresJobStore::enqueue_in_transaction`, and commit once. When a handler writes its result in PostgreSQL, write the result and call `PostgresJobStore::complete_in_transaction` last in the same transaction.

Pass the attempt's `JobCancellation::worker_id()` to that completion call.
After the transaction commits, call
`JobCancellation::mark_completed_in_transaction()` and return success
immediately so the runner records the attempt without issuing a second
completion transition.

External side effects still require a provider idempotency key. A worker can lose its lease after the provider accepted the call but before local completion committed.

An inbox key must include the full idempotency scope. Use at least owner, source
or connection, and source event ID. Put a unique constraint on that tuple. A
globally unique event ID is not enough because two owners or sources may use the
same identifier. Store a digest of the signed or validated payload if the
sender can reuse an ID with different content. Reject that conflict instead of
replaying an unrelated outcome.

Process one delivery as follows:

1. Start a transaction and establish the owner from authenticated context.
2. Insert the inbox row with the scoped key and validated payload. On a unique
   conflict, lock and read the committed row.
3. Apply the product's domain operation.
4. Insert each durable consequence into the outbox.
5. Store the complete sender-facing outcome on the inbox row.
6. Commit, then return the stored outcome.

PostgreSQL makes a concurrent unique-key insert wait for the other transaction.
After the winner commits, the waiting call must read and return its outcome. If
the winner rolls back, the waiting call may become the first delivery. Do not
return a placeholder outcome that was written before domain processing.

An expected domain rejection may commit as a final inbox outcome when replaying
that rejection is part of the product protocol. A storage error, injected
failure after inbox insertion, failed domain write, or failed outbox write must
roll back the inbox row and every effect. A later delivery can then try again as
the first delivery. When the inbox and outbox use different durable systems,
commit the accepted inbox outcome first and use the reconciler described below.

For inbound webhooks:

1. Read the bounded raw body needed by the signature scheme.
2. Verify signature, timestamp/replay window, and expected provider/tenant context before persistence.
3. Deduplicate on a stable delivery identifier.
4. In one PostgreSQL transaction, persist the verified inbox row and enqueue its outbox job.
5. Return an accepted response only after the transaction commits.

If enqueue uses another system or can fail independently, the accepted inbox must be the durable source. A bounded drainer/reconciler claims accepted-but-not-enqueued rows, retries transfer idempotently, and exposes age, attempts, and terminal state. An in-memory handoff after returning success can silently lose accepted delivery and is forbidden.

Run `baukit_test::check_postgres_inbox_conformance` through a small adapter over
the product's real PostgreSQL schema. It covers first delivery, exact replay,
concurrent replay, rollback after inbox insertion, domain failure, outbox
failure, owner and source isolation, and replay after process-local state is
discarded. The Baukit suite runs the concurrent case against PostgreSQL 18 in
Docker. Product tests must do the same against their migrations.

## 5. Outbound webhook delivery

Fan out an event into one durable job for each matching subscription. Use the
subscription ID and event ID together as the job idempotency key. A job sends
to one URL only. Do not put several subscriptions in one retryable job because
one failed target would cause the next attempt to resend to targets that
already returned success.

Give every delivery a stable delivery ID derived from or stored with the
subscription and event. Send the same ID and the exact same body on every
attempt. Receivers must claim that ID before applying their domain effect and
return their stored outcome on replay. A sender timeout means the receiver may
have committed, so retrying without receiver idempotency is unsafe.

Sign the raw request body with a timestamp and delivery ID. The
`baukit_test::webhook_signing_input` format is:

```text
baukit-webhook-v1\n
<unix timestamp>\n
<delivery ID byte length>\n
<delivery ID>\n
<raw body bytes>
```

`sign_webhook_hmac_sha256` returns an HMAC-SHA256 value with a `v1=` prefix and
unpadded base64url data. Send the signature version, a non-secret key ID, Unix
timestamp, and delivery ID in separate headers. The receiver must check the
timestamp against a bounded replay window, compare the HMAC in constant time,
and claim the delivery ID in its inbox. Parse or re-encode JSON only after
verification because either operation can change the signed bytes.

Generate a new random secret for rotation and store it with a new key ID. Sign
new jobs only with the current key. During a documented overlap, receivers may
verify the current and previous keys selected by key ID. Remove the old key
after the longest job retry window and clock-skew allowance have passed. Keep
secrets in `baukit-credential-vault`; never return them after creation or put
them in logs, metrics, job errors, or receiver fixtures.

Classify each attempt from the HTTP result:

| Result | Delivery action |
|---|---|
| Any `2xx` | Mark the one delivery successful and stop. |
| Timeout or connection loss | Retry because the receiver outcome is unknown. |
| `408`, `425`, `429`, or `5xx` | Retry within the job attempt cap. Honor a valid bounded `Retry-After`. |
| `3xx` | Do not follow the redirect. Record a permanent failure for this delivery. |
| Other `4xx` | Record a permanent failure for this delivery. Do not retry the same body. |

Set finite connect, request, and response-body limits. Discard receiver bodies
after classification. Store bounded machine codes, not response text. The
product must state its disable policy separately from job retries. A useful
policy counts consecutive terminal or exhausted delivery jobs per subscription,
resets the count after a success, and disables at a configured threshold.
Count one failed job, not each attempt. Products may disable `404` and `410`
sooner, but the threshold and re-enable flow are product policy.

Treat subscription URLs as untrusted network input. Require HTTPS outside an
explicit local-development mode. Reject user information, fragments, invalid
ports, oversized values, and non-HTTP schemes. Resolve the host for every
delivery attempt and reject every loopback, private, link-local, multicast,
unspecified, reserved, and deployment metadata address. Connect only to an
approved resolved address while preserving the original hostname for TLS and
the `Host` header. Do not follow redirects. Apply the same checks to every
address returned by DNS, enforce outbound network policy, and redact URL query
values from diagnostics. A product that cannot implement and test these rules
must use an allowlist or an outbound proxy that enforces them.

`ScriptedWebhookReceiver` supplies bounded loopback requests and queued
responses for success, rate limit, permanent failure, timeout, and retry tests.
Use the signing helpers to assert that retries keep the same body, timestamp,
delivery ID, and signature. The fixture deliberately has no `Debug`
implementation for captured requests.

Migration from a multi-target job is additive but changes delivery behavior.
Create per-subscription jobs transactionally before switching the worker. Keep
the old payload handler until existing jobs drain, and give new jobs a distinct
type or payload version. If the signature input changes, receivers must accept
both versions during the rotation overlap. Preserve existing delivery IDs so
receiver inboxes still recognize retries.

A future `baukit-webhooks` crate requires two products with the same
subscription and delivery model and a reviewed, complete server-side
request-forgery policy. Until then, subscription schema, URL policy, headers,
disable thresholds, event bodies, and job payloads remain product-owned.

## 6. Batches and client queues

Prefer one external consequence per durable job. When a provider requires batching, record an outcome for each item and retry only retryable items; a single malformed or revoked item must not discard successful siblings or cause the whole batch to appear successful.

Client delivery queues need both size and attempt bounds. Persist an idempotency key, next attempt, attempt count, and inspectable terminal code. Define overflow and discard behavior explicitly. “Delivered” or “synced” is false while hidden pending or terminal items remain.

## 7. Stable failures and localized recovery

Map provider/transport failures to stable snake_case machine codes at the product boundary, for example `provider_rate_limited`, `provider_unavailable`, `provider_revoked`, or `delivery_exhausted`. APIs return those codes with safe structured details. Web and mobile map `code + details` to localized copy and recovery actions; they never display `last_error` or an exception string.

`@baukit/integrations-client` maps server health and client events to fixed connection states and
available actions. Its output contains no display text. Products map those states to localized copy
and must not render the machine diagnostic code. Raw provider diagnostics are discarded.

The same package coordinates OAuth sessions through injected browser, native, redirect, storage,
timer, nonce, and clock ports. Allow a callback only when its origin and path match the configured
allowlist exactly and its state nonce matches the stored in-flight session. Discard stale sessions.
Reserve a web popup before awaiting the server's authorization URL so the browser still associates
the popup with the user's click.

Products continue to own provider consent parameters, scopes, persistence, identity, localized copy,
and recovery policy. Register the product's provider definitions once. Each definition may carry a
typed product connector, including its OAuth starter, hooks, and icon. Apply current server state
with `withConnectionStates` when a query or local cache changes. The method returns a new registry;
the definitions and their registration order remain unchanged. A provider missing from the state
collection is `disconnected` with no actions. Do not add provider branches to the shared package.

## 8. Deterministic fakes and acceptance checks

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

Credential-probe acceptance adds healthy account identity, revoked, missing
scope, rate limited with and without `Retry-After`, timeout, unavailable,
malformed data, and an oversized response. Run the same conformance function
for each provider adapter. Only the raw scripted responses should differ.

Inbox acceptance adds first delivery, exact and concurrent replay, rollback at
each transaction boundary, explicit owner and source isolation, and durable
outcome replay. Outbound webhook acceptance adds one job per subscription and
event, stable retry bytes and delivery IDs, signing-key overlap, every HTTP
class above, disable thresholds, response bounds, and URL rejection for each
forbidden address class.

Run unit tests plus PostgreSQL integration tests with Docker-gated ignored tests enabled. Exercise worker telemetry conformance and generated worker fixtures whenever shared APIs or templates change.

## 9. Telemetry bounds and adoption

Reuse Baukit's `worker_job_runs_total`, `worker_job_duration_seconds`, and `worker_queue_oldest_age_seconds`. Additional product metrics must be registered and use bounded code-defined labels such as provider, operation, job, and outcome enums. Never use secrets, user/connection IDs, URLs, payload fields, exception text, request IDs, or other dynamic strings as labels.

Fitness-style worker slices should migrate onto `baukit-jobs` or a thin product adapter around it. Record real missing primitives during that adoption before extending Baukit. Provider models, OAuth behavior, webhook payloads, recipe imports, and food databases remain in their products.

Push delivery is the one that moved: `baukit-push` owns the `PushSender` port and the Expo ticket/receipt adapter, including the `DeviceNotRegistered` signal products use to prune dead tokens. Scheduling, quiet hours, and deciding who gets notified stay product-side.
