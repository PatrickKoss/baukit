# ADR 0032: Push goes through a provider-neutral port, device tokens move between owners only with an install secret, and scheduled pushes take a delivery claim

## Status

Accepted, 2026-11-27.

## Context

Products send reminders and alerts to phones. Expo is the provider today, but
domain code that names Expo would have to change with the provider. Expo also
confirms delivery late: the send returns a ticket, and the real result comes
from a receipt poll minutes later.

A device token is a bearer value. If any signed-in account could register a
token another account holds, it could take over that device's notifications.
Scheduled senders run on several workers, and two of them can pick the same
owner for the same daily reminder.

## Decision

`baukit-push` is opt-in. The generated backend does not include it.

- `PushSender` takes a batch of `PushMessage` values and returns `PushOutcome`
  values. `ExpoPushSender` is the only adapter. Above the port nothing names
  Expo. `PushDeliveryStatus` is `Delivered`, `Rejected(PushRejection)` or
  `Accepted(PushTicketId)`. `Accepted` means in flight and is never resent.
  `PushReceiptSource` and `PendingReceiptStore`, with
  `PostgresPendingReceiptStore`, poll tickets later, after `RECEIPT_POLL_DELAY`.
- `DeviceRegistry` stores tokens per owner. `PostgresDeviceRegistry` caps each
  owner at `DEFAULT_DEVICES_PER_OWNER` and evicts the oldest. `dead_tokens`
  picks `DeviceNotRegistered` tokens out of outcomes for removal.
- Each install sends a `DeviceInstallSecret` of 16 to 256 characters through
  `DeviceRegistration::with_install_secret`. The registry stores only its
  SHA-256 digest. `takeover_decision` returns `Keep`, `Move`, `MoveLegacy` or
  `Refuse`. A token held by another owner moves only when the registering
  owner presents the same secret. A row without a digest moves as before and
  logs a warning. `Refuse` surfaces as
  `PushStoreError::TokenHeldByAnotherOwner`, which products answer with 409.
  `DeviceInstallSecret` does not derive `Debug`.
- `DeliveryClaimStore`, with `PostgresDeliveryClaimStore`, keeps a scheduled
  push at most once per owner, local date and `DeliveryKind`. `claim` returns
  `Some(ClaimToken)` to the single winner. `release(claim, token)` frees it
  after a failed send only while that token still holds it, so a stale token
  releases nothing. `purge` deletes old claims in bounded batches.
- Who gets notified, when, the copy, quiet hours and channels stay in the
  product.

The tables come from the reference SQL in `rust/crates/baukit-push/migrations/`,
as [ADR 0026](0026-reference-sql-migrations.md) describes.

## Enforced by

- `rust/crates/baukit-push/tests/registry_model.rs`:
  `a_token_moves_to_another_owner_only_with_its_install_secret`, and
  `postgres_registry_matches_the_memory_registry`, a proptest of the PostgreSQL
  registry against the in-memory one.
- `rust/crates/baukit-push/tests/postgres.rs`:
  `a_daily_claim_has_exactly_one_winner_until_released`,
  `old_claims_purge_in_bounded_batches`,
  `a_shared_token_ends_with_exactly_one_owner`,
  `concurrent_registrations_never_overshoot_the_cap` and
  `concurrent_pollers_take_disjoint_batches`.
- `rust/crates/baukit-push/tests/telemetry_redaction.rs` checks that tokens do
  not reach logs.
- These run in the `rust-tests` gate step with `--include-ignored`.
- Nothing stops a product from calling Expo directly. The
  `rust-raw-http-client` ast-grep rule catches a raw `reqwest::Client`, but not
  a call through `baukit-egress`.

## Consequences

Products can swap providers behind `PushSender` and get takeover protection
and at-most-once scheduled pushes from the store. They must generate and keep
the install secret in the app's secure storage. Rows registered before
install secrets can still be taken over until their next registration. Claims
grow one row per owner, kind and day, so products schedule `purge`.

## History

- 2026-08-25, `af6e2b4f`: baukit 0.1.0 shipped the push port and Expo adapter.
- 2026-11-25, `fdbfc03a`: token takeover became fenced by the install secret.
  Before that a token moved to any owner that registered it.
- 2026-11-25, `0e19e4a2`: `claim` returned `bool` before. It now returns a
  `ClaimToken`, and `release` needs that token.
