# Item 26 evidence: inbox and webhook reliability

## Source product files

- `/home/patrick/projects/tiefgang/backend/migrations/20260824000005_suite.sql`
- `/home/patrick/projects/tiefgang/backend/crates/tiefgang-postgres/src/events_ingest.rs`
- `/home/patrick/projects/tiefgang/backend/crates/tiefgang-postgres/src/webhooks.rs`
- `/home/patrick/projects/tiefgang/backend/crates/tiefgang-worker/src/lib.rs`
- `/home/patrick/projects/tiefgang/backend/crates/tiefgang-services/src/webhooks.rs`
- `/home/patrick/projects/tiefgang/backend/tests/postgres_integration.rs`
- `/home/patrick/projects/tiefgang/backend/tests/worker_integration.rs`

## Observed failure or repeated glue

Tiefgang serializes inbox processing, stores replay outcomes, applies domain
writes, and enqueues outbound work in PostgreSQL. Its inbox primary key is only
`event_id`, so unrelated owners and sources cannot safely reuse an ID. Its
webhook worker sends one event to several subscriptions in one job. If one
target asks for a retry after another succeeds, the next attempt sends again to
the successful target.

## Baukit owner

`docs/platform/integration-reliability.md` owns the recipes.
`baukit-test` owns product-neutral conformance and HTTP fixtures. Product inbox
schemas and webhook delivery code remain local.

## Public types and errors

Inbox types are `InboxScope`, `InboxDelivery`, `InboxDisposition`,
`InboxReceipt`, `InboxState`, `InboxFault`, `PostgresInboxPort`,
`InboxConformanceCases`, and `InboxConformanceError`. Webhook helpers are
`webhook_signing_input`, `sign_webhook_hmac_sha256`,
`verify_webhook_hmac_sha256`, `ScriptedWebhookReceiver`,
`ScriptedWebhookResponse`, and `ReceivedWebhookRequest`. The conformance error
contains fixed violation text and never formats the product adapter error.

## Product-owned inputs

Products own owner and source identifiers, event IDs and payloads, inbox and
outbox schemas, stored outcomes, domain errors, subscription tables, event
bodies, header names, secrets, URL policy, retry caps, and disable thresholds.

## Concurrency, failure, privacy, and cleanup cases

The inbox suite covers first delivery, exact replay, two concurrent deliveries,
rollback after inbox insertion, domain failure, outbox failure, owner and source
isolation, and outcome replay after transient state is cleared. Webhook tests
can script success, retry, permanent failure, and timeout. Inbox values and
captured requests do not implement `Debug`; response bodies and secrets do not
enter errors. Product retention and erasure must remove owner-scoped inbox,
outbox, delivery, and secret rows in product-defined order.

## Supported runtimes

The helpers support Rust 1.95 on Tokio. Inbox conformance targets PostgreSQL and
the Baukit test uses PostgreSQL 18 in Docker. The scripted receiver binds an
ephemeral loopback TCP port.

## Product adoption change

A Tiefgang adoption change should replace its inbox replay assertions with
`check_postgres_inbox_conformance`, migrate inbox uniqueness to owner plus source
plus event ID, and split `events.deliver` into one job per subscription and
event. It can then delete the multi-target delivery loop in
`backend/crates/tiefgang-worker/src/lib.rs` and the duplicate signature helper
in `backend/tests/worker_integration.rs`. The product worker still owns its
production signer and HTTP delivery code.
