# Changelog

All notable changes to `baukit-push` are documented here.

## [Unreleased]

## [0.10.9] - 2026-10-10

## [0.10.8] - 2026-10-09

## [0.10.7] - 2026-10-09

## [0.10.6] - 2026-10-09

## [0.10.5] - 2026-10-08

## [0.10.4] - 2026-10-08

## [0.10.3] - 2026-10-08

## [0.10.2] - 2026-10-08

## [0.10.1] - 2026-10-08

## [0.10.0] - 2026-10-08

## [0.9.0] - 2026-10-07

## [0.8.0] - 2026-10-07

## [0.7.4] - 2026-10-06

## [0.7.3] - 2026-10-05

## [0.7.2] - 2026-10-04

## [0.7.1] - 2026-10-04

## [0.7.0] - 2026-10-04

### Changed

- Use caret requirements for third-party Rust dependencies so products can take compatible
  updates. Keep Baukit crate versions exact.

### Fixed

- Select aws-lc-rs for sqlx TLS so rustls has one crypto provider. Products
  must use sqlx `tls-rustls-aws-lc-rs` instead of `tls-rustls`, including dev
  dependencies, and disable the `ring` default in testcontainers and
  testcontainers-modules.

## [0.6.0] - 2026-10-02

### Changed

- Moved to reqwest 0.13 with its `rustls` backend. HTTPS clients now verify servers against the
  operating system's trust store through rustls-platform-verifier instead of the bundled
  webpki roots, so a container image needs CA certificates (distroless `cc` ships them). The
  TLS crypto provider is aws-lc-rs instead of ring.

## [0.5.2] - 2026-09-30

### Added

- Pending receipts record their owner. Migration
  `0004_baukit_push_pending_receipt_owners.sql`
  (`POSTGRES_PUSH_PENDING_RECEIPT_OWNERS_MIGRATION_SQL`) adds a non-null
  `owner_id` column to `push_pending_receipts`, and its header shows the owner
  foreign key with `ON DELETE CASCADE`. Erasing an owner now removes the device
  tokens their pending tickets hold at once instead of up to 24 hours later.
- Add `PendingReceiptStore::erase_owner`, implemented by
  `PostgresPendingReceiptStore` and `MemoryPendingReceiptStore`, and
  `erase_owner_pending_receipts`, which takes any `PgExecutor` for the
  product's erasure transaction.

### Breaking

- `PendingReceipt` has a new `owner_id` field.
- `PendingReceiptStore::record_accepted` and `accepted_receipts` take the
  owner as their first argument.
- `PendingReceiptStore` has a new required method, `erase_owner`. Custom
  implementations must add it.
- Migration `0004` deletes every pending ticket recorded before it runs. A
  dead token among them is reported again by the next send to it.

## [0.5.1] - 2026-09-29

### Added

- Add `DeliveryClaimStore::purge(before, limit)`, which deletes one batch of
  claims for local dates before `before`, oldest date first, and returns how
  many went. `PostgresDeliveryClaimStore` runs `purge_delivery_claims` on its
  pool, and `MemoryDeliveryClaimStore` purges in the same order, so scheduled
  purge jobs no longer need a product repository method or a store they cannot
  fake in tests.

### Breaking

- `DeliveryClaimStore` has a new required method, `purge`. Custom
  implementations must add it.

## [0.5.0] - 2026-09-28

### Added

- Add the `DeviceRegistry` port with `register`, `rotate`, `unregister`,
  `list_for_owner`, `invalidate`, and `erase_owner`, plus the provided
  `invalidate_dead_tokens`, which removes every `DeviceNotRegistered` token
  from a send's outcomes in one call. A token registered again after the send
  survives. Registering a token another owner holds moves it. Each owner keeps
  at most `DEFAULT_DEVICES_PER_OWNER` (10) devices; a registration over the cap
  evicts the least recently registered, tie-broken by `created_at` and then the
  token, and never the device being registered. `rotate` removes the
  predecessor first, so it never evicts another device.
- Add `DeviceToken`, whose `Debug` output is redacted, `DeviceTimeZone`,
  `DevicePlatform`, `DeviceRegistration`, `RegisteredDevice`,
  `RegistrationOutcome`, `dead_tokens`, `PushValidationError`,
  `PushStoreError`, `PushStoreFuture`, `MAX_DEVICE_TOKEN_LENGTH` (512), and
  `MAX_DEVICE_TIME_ZONE_LENGTH` (64). No error text carries a token.
- Add the `DeliveryClaimStore` port with `claim` and `release`, keyed by
  `DeliveryClaim` (owner, local date, `DeliveryKind`), for scheduled senders.
  It is separate from the registry, so event-driven senders ignore it.
- Add the `sqlx-postgres` feature with `PostgresDeviceRegistry` over
  `POSTGRES_PUSH_DEVICES_MIGRATION_SQL` and `PostgresDeliveryClaimStore` over
  `POSTGRES_PUSH_DELIVERY_CLAIMS_MIGRATION_SQL`. Both constants are available
  without the feature. Registration runs under a per-owner advisory lock, so
  concurrent registrations cannot overshoot the cap. Add
  `erase_owner_push_devices`, `erase_owner_delivery_claims`, and the batched
  `purge_delivery_claims`.
- Add `MemoryDeviceRegistry` and `MemoryDeliveryClaimStore` behind
  `test-support`.
- `baukit-push` now depends on `chrono` and `uuid`.
- Add deferred receipt polling. `PushTicketId` (1 to
  `MAX_PUSH_TICKET_ID_LENGTH` (128) bytes of visible ASCII) names an accepted
  notification. The `PushReceiptSource` port returns settled `PushReceipt`
  values per ticket, and `ExpoPushSender` implements it, splitting requests at
  `MAX_RECEIPT_BATCH_SIZE` (1000, Expo's limit). The `PendingReceiptStore` port
  records `PendingReceipt` values (ticket, token, `sent_at`), takes a bounded
  due batch and moves it back by `RECEIPT_POLL_DELAY` (15 minutes), deletes
  settled tickets, and purges tickets older than `RECEIPT_RETENTION` (24
  hours). Its provided `record_accepted` records every accepted outcome of a
  send. `poll_pending_receipts` takes one due batch, fetches receipts,
  invalidates `DeviceNotRegistered` tokens with each send's own `sent_at`,
  deletes the settled tickets, and returns a `ReceiptPoll`; failures are a
  `ReceiptPollError`. `accepted_receipts` is the filter behind
  `record_accepted`. There is no scheduler; products drive the poll from their
  job runner.
- Add `PostgresPendingReceiptStore` behind `sqlx-postgres` over
  `POSTGRES_PUSH_PENDING_RECEIPTS_MIGRATION_SQL`, and `MemoryPendingReceiptStore`
  behind `test-support`. `FakePushSender` implements `PushReceiptSource` and
  gains `settle_receipt` and `receipt_requests`.

### Changed

- **Break:** `PushMessage::token` and `PushOutcome::token` are `DeviceToken`
  instead of `String`, so their `Debug` output redacts the token.
  `PushMessage::new` takes a `DeviceToken`. `FakePushSender::reject` and
  `accept_without_receipt` take a `DeviceToken`, and
  `FakePushSender::dead_tokens` returns `Vec<DeviceToken>`. `dead_tokens` no
  longer skips invalid tokens, since an outcome can no longer hold one.
- **Break:** `PushDeliveryStatus::Accepted` carries the provider's
  `PushTicketId`.
- **Break:** `PushValidationError` gains the `TicketId` variant.
- **Break:** `FakePushSender::fail_with` also fails receipt requests.

## [0.4.0] - 2026-09-12

## [0.3.0] - 2026-09-04

## [0.2.1] - 2026-09-03

## [0.2.0] - 2026-09-03

## [0.1.2] - 2026-09-01

## [0.1.1] - 2026-09-01

## [0.1.0] - 2026-08-25

### Added

- First public release of `baukit-push`.
