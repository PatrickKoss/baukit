# Changelog

All notable changes to `baukit-push` are documented here.

## [Unreleased]

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

## [0.4.0] - 2026-09-12

## [0.3.0] - 2026-09-04

## [0.2.1] - 2026-09-03

## [0.2.0] - 2026-09-03

## [0.1.2] - 2026-09-01

## [0.1.1] - 2026-09-01

## [0.1.0] - 2026-08-25

### Added

- First public release of `baukit-push`.
