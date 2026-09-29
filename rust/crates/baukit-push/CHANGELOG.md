# Changelog

All notable changes to `baukit-push` are documented here.

## [Unreleased]

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
