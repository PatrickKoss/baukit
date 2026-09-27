# Push device registry evidence

Plan item 17, "Add a push device registry". The comparison, the registry port, receipt-driven
invalidation, the separate daily delivery claim, the optional PostgreSQL store, and the Docker-backed
tests are implemented in `baukit-push`. Product adoption is deferred.

## Source revisions

- Baukit baseline `a5227a7`.
- Eigenruhe `f74cebb`.
- Hebkit `841bf5d`.
- Leitbild `bd38b33`.

## Step 1: three registries compared

| Point | Eigenruhe | Hebkit | Leitbild |
|---|---|---|---|
| Table | `device_push_tokens` | `device_push_tokens` | `push_tokens` |
| Identity | `id UUID` PK, `UNIQUE (owner_id, token)` | `id uuid` PK, `UNIQUE (user_id, expo_push_token)` | PK `(user_id, installation_id)` |
| Token limit | `char_length <= 4096` | none | 1 to 512 characters |
| Platform | `ios`, `android` | `ios`, `android` | `ios`, `android` |
| Time zone | `timezone NOT NULL DEFAULT 'UTC'`, at most 64 | `timezone NOT NULL` | none; kept in `reminder_settings` |
| Timestamps | `created_at`, `last_seen_at` | `created_at`, `last_seen_at`, `updated_at` | `created_at`, `updated_at` |
| Owner join | FK `user_identities` cascade | FK `users` cascade | FK `user_identities` cascade |
| Cap per user | 10, rejects the eleventh with `row_cap_per_user` | 10, evicts beyond the newest 10 | 10, evicts the oldest before insert |
| Cap order | none | `last_seen_at DESC, created_at DESC, id DESC`, keep 10 | `updated_at, installation_id`, evict first |
| Cap locking | `SELECT ... FOR UPDATE` on the owner row | same | same |
| Rotation | client registers the new token, then unregisters the old one in a second call | same | same installation replaces its token in place |
| Unregister | owner and token | owner and token | owner and installation |
| Dead tokens | `DELETE ... WHERE owner_id = $1 AND token = ANY($2)` after each send | `DELETE ... WHERE expo_push_token = ANY($1)` across owners | never pruned; the sender turns the rejection into a job error |
| Daily claim | `notification_log (owner_id, kind, period)` unique, released on send failure | `push_notification_deliveries (user_id, local_date, kind)` unique, released on send failure | none; the job dedupe key `daily-reminder:{user}:{local_day}` |
| Erasure | cascade, plus an explicit delete in the profile erasure list | cascade | cascade |

Sources:

- Eigenruhe: `backend/migrations/20260823000000_sync_foundation.sql:73-83`,
  `20260823000007_notifications.sql` (renames the table and adds `timezone` and `last_seen_at`), `20260903000000_quota_constraints.sql:22-24`, and
  `limits.json:13,40`. Adapter `backend/crates/eigenruhe-postgres/src/notifications.rs:15-166`.
  Service `backend/crates/eigenruhe-services/src/notifications.rs:84-121` (register and
  unregister) and `:254-316` (claim, send, release, prune). Erasure lists in
  `eigenruhe-domain/src/profile_erasure.rs:5,23` and `eigenruhe-postgres/src/profile.rs:32,119`.
  Mobile `mobile/src/features/reminders/push-registration.ts` and `push-token-device.ts`, sign-out
  in `mobile/src/features/auth/sign-out.ts:9`.
- Hebkit: `backend/migrations/20260827100000_core.sql:81-107` and `limits.json:86`. Port
  `backend/crates/hebkit-ports/src/notification_repository.rs:17-53`. Adapter
  `backend/crates/hebkit-postgres/src/adapters/postgres/notifications.rs:62-251`. Service
  `backend/crates/hebkit-services/src/notifications.rs:62-170`. Mobile
  `mobile/src/features/reminders/registration.ts` and `registration-adapter.ts`.
- Leitbild: `backend/migrations/0006_add_reminders_and_streak_settings.sql:18-29`, the default
  cap in `backend/crates/leitbild-domain/src/limits.rs:271`. Port
  `backend/crates/leitbild-ports/src/lib.rs:276-286`. Adapter
  `backend/crates/leitbild-postgres/src/reminder.rs:152-250`. Worker
  `backend/crates/leitbild-worker/src/lib.rs:196-224`. Sender
  `backend/crates/leitbild-notifications/src/lib.rs:47-58`. Mobile `mobile/src/push-registration.ts`.

### Decisions

**The token is the identity, and one token has one owner.** Eigenruhe and Hebkit key on
`(owner, token)`, so the same device token can sit under two owners. Both clients unregister on
sign-out, but best-effort: Eigenruhe swallows the failure (`sign-out.ts:9`) and Hebkit's comment
says the receipt path will clean up. A token only dies when the app is uninstalled, so a device
that switches accounts while offline keeps receiving the previous account's notifications. Baukit
makes the token the primary key and moves it on registration. Hebkit's receipt eviction already
treats the token as global, which only works if the token is unique.

**No installation ID.** Only Leitbild keys on an installation, and its mobile code never calls
`registerPushToken`. The two products that ship registration keep the last registered token in
secure storage and send it back on rotation. Baukit takes that shape and makes it one call:
`rotate(previous, registration)` deletes the predecessor and registers the new token in one
transaction. Today both products call register and then unregister. With Eigenruhe's reject cap,
an owner at ten devices cannot rotate at all, because the new token is refused before the old one
goes. With Hebkit's eviction cap, the new token can evict a live device before the stale
predecessor is removed.

**Evict, do not reject, and evict the least recently registered.** Two of three products evict.
Rejecting strands the user's newest phone, which is the device most likely to be in use. The
order is `last_registered_at`, which every registration moves forward (Hebkit's `last_seen_at`
and Leitbild's `updated_at` mean the same thing). Ties break on `created_at` and then on the token,
so two stores given the same history evict the same row. The token being registered is excluded
from eviction, so a server with a slow clock cannot evict the device it is registering. The
default cap is 10, the value all three products chose, and `with_devices_per_owner` changes it.

**Invalidation needs no owner and respects later registrations.** A `DeviceNotRegistered` verdict
is about the token, so `invalidate` deletes by token alone. It deletes only rows with
`last_registered_at <= sent_at`. A device that registered again after the send started keeps its
token, because Expo's verdict predates the new registration. This matters when a user reinstalls
or re-enables notifications while a receipt is outstanding.

**Time zone is optional registration data.** Eigenruhe and Hebkit store the device's time zone and
schedule by it; Leitbild keeps it in settings. Baukit stores an optional `time_zone` with an
IANA-shaped check (1 to 64 bytes of letters, digits, `/`, `_`, `+`, `-`). It does not resolve the
name. Products that schedule by it validate it with their time zone database first.

**Token limit 512 bytes of visible ASCII.** Expo tokens are about 41 characters, FCM registration
tokens about 160, APNs tokens 64 hex digits. Eigenruhe's 4096 is its generic text cap, and a B-tree
primary key cannot hold values near that size. Leitbild's 512 fits every provider.

**The daily claim is a separate port keyed by owner, local date, and kind.** This is Hebkit's
shape. Eigenruhe's weekly kinds key on an ISO week string and map to the week's Monday as the local
date. Eigenruhe's `connection_reattention` period is `{connection_id}:{updated_at in microseconds}`, which is event
state rather than a calendar day; it stays a product table. Leitbild's reminder already dedupes
through the `baukit-jobs` idempotency key and does not need the claim. Both products that claim
release on a failed send, so `release` is part of the port. Neither product ever deletes old
claims, which grow by one row per owner, kind, and day; `purge_delivery_claims` deletes them in
bounded batches.

**No conformance helper in `baukit-test`.** Products adopt `PostgresDeviceRegistry` rather than
keeping their own adapters, so nothing outside Baukit would run a conformance suite. The Docker
tests live in `rust/crates/baukit-push/tests/postgres.rs`, and the in-memory fakes carry unit tests
for the same rules.

## Baukit owner

`rust/crates/baukit-push`. The port types live in the base crate. The PostgreSQL adapters sit
behind the optional `sqlx-postgres` feature, following `baukit-auth` and `baukit-sync`: optional
`sqlx`, reference migrations in `migrations/`, and `*_MIGRATION_SQL` constants available without
the feature. The in-memory fakes sit behind the existing `test-support` feature beside
`FakePushSender`.

## Public types and functions

- `DeviceRegistry` with `register`, `rotate`, `unregister`, `list_for_owner`, `invalidate`,
  `erase_owner`, and the provided `invalidate_dead_tokens`.
- `DeviceToken` (`new`, `expose`, redacted `Debug`), `DeviceTimeZone`, `DevicePlatform`,
  `DeviceRegistration`, `RegisteredDevice`, `RegistrationOutcome`, and `dead_tokens`.
- `DeliveryClaimStore` with `claim` and `release`, `DeliveryClaim`, `DeliveryKind`.
- `PushValidationError`, `PushStoreError`, `PushStoreFuture`.
- `DEFAULT_DEVICES_PER_OWNER` (10), `MAX_DEVICE_TOKEN_LENGTH` (512),
  `MAX_DEVICE_TIME_ZONE_LENGTH` (64), `MAX_DELIVERY_KIND_LENGTH` (64).
- `POSTGRES_PUSH_DEVICES_MIGRATION_SQL` and `POSTGRES_PUSH_DELIVERY_CLAIMS_MIGRATION_SQL`.
- Behind `sqlx-postgres`: `PostgresDeviceRegistry` (`new`, `with_devices_per_owner`),
  `PostgresDeliveryClaimStore`, `erase_owner_push_devices`, `erase_owner_delivery_claims`, and
  `purge_delivery_claims`.
- Behind `test-support`: `MemoryDeviceRegistry` and `MemoryDeliveryClaimStore`.

No wire type was added. The registration request body stays a product route; a product that
adopts should name its fields in camelCase per the plan's naming convention.

## Contract as implemented

Registry, PostgreSQL:

- `push_devices` has `token TEXT PRIMARY KEY`, `owner_id UUID`, `platform`, optional `time_zone`,
  `created_at`, and `last_registered_at`, with CHECKs matching `DeviceToken` and `DeviceTimeZone`
  and `last_registered_at >= created_at`. The index `(owner_id, last_registered_at DESC,
  created_at DESC, token DESC)` serves the list and the eviction scan.
- `register` and `rotate` open one transaction, take
  `pg_advisory_xact_lock(hashtextextended('baukit_push.devices:' || owner_id, 0))`, delete the
  owned predecessor when rotating, upsert, and evict. The advisory lock needs no owner table.
- The upsert is `INSERT ... ON CONFLICT (token) DO UPDATE`. For the same owner it keeps
  `created_at` and sets `last_registered_at` to the later of the two instants, so a late refresh
  never moves it back. For a different owner it moves the row and resets both instants.
- Eviction deletes `WHERE owner_id = $1 AND token IN (... ORDER BY last_registered_at DESC,
  created_at DESC, token DESC OFFSET cap - 1)`, excluding the registered token. The outer
  `owner_id` predicate is rechecked after a row lock wait, so a token that moved to another owner
  mid-flight is not evicted from its new owner.
- `unregister` matches owner and token. `invalidate` is one `DELETE ... WHERE token = ANY($1) AND
  last_registered_at <= $2`. `erase_owner` and `erase_owner_push_devices` delete by owner.

Registry, port level:

- `invalidate_dead_tokens` keeps outcomes where `PushOutcome::is_token_dead` is true, skips tokens
  that fail `DeviceToken::new`, deduplicates, and makes one `invalidate` call. With no dead token
  it returns `Ok(0)` without touching the store.

Claims:

- `push_delivery_claims` has primary key `(owner_id, local_date, kind)` and `claimed_at`.
  `claim` is `INSERT ... ON CONFLICT DO NOTHING` and returns whether it inserted. `release` deletes
  and returns whether a row went.
- `purge_delivery_claims(executor, before, limit)` deletes one batch with `local_date < before`,
  `FOR UPDATE SKIP LOCKED`, and returns the count.

## Cases

Docker-backed, in `rust/crates/baukit-push/tests/postgres.rs`, each on a fresh PostgreSQL
container with both reference migrations and a test owner table joined by cascading foreign keys:

- Register, refresh, unregister: a 512-byte token round-trips; a refresh updates platform and time
  zone, keeps `created_at`, and a late refresh does not move `last_registered_at` back. Another
  owner's unregister returns `false` and leaves the device.
- Deterministic cap eviction: with a cap of three and two devices registered at the same instant,
  the lower token goes first. A registration with an older clock is kept and the oldest other
  device goes. Another owner is untouched.
- Concurrent cap: 16 concurrent registrations against a cap of three leave exactly three devices,
  and the reported evictions add up to 13. With the advisory lock removed, this test failed in two
  of three runs.
- Rotation: rotating at the cap evicts nothing. Two concurrent rotations from the same predecessor
  both succeed, the predecessor is gone, and the owner stays at the cap. A retried rotation is a
  refresh.
- Shared token: eight owners registering one token concurrently leave exactly one row and one
  holder. A later registration by another owner moves it with fresh timestamps.
- Receipt invalidation: one `invalidate_dead_tokens` call removes a dead token, ignores a
  `MessageTooBig` rejection, an unknown token, and a duplicate, and keeps a token registered
  again after `sent_at`. A second call removes nothing.
- Erasure: `erase_owner` through the port, `erase_owner_push_devices` and
  `erase_owner_delivery_claims` inside a transaction, and deleting the owner row each remove
  exactly that owner's rows.
- Claims: ten concurrent claims give one winner. Another kind and another date are independent.
  Release then allows a new claim; a second release returns `false`.
- Purge: batches of one delete the two claims before the cutoff and stop, keeping the cutoff date
  and later.

Unit tests cover token, time zone, and kind validation, the redacted `Debug` of tokens and
registrations, the dead-token filter, and the in-memory fakes' cap, rotation, move, invalidation,
erasure, and claim rules.

## Failure behavior

- Invalid tokens, time zones, and kinds fail at construction with `PushValidationError`, whose
  display names the field and never the value.
- Every SQL failure becomes `PushStoreError::Internal` with the fixed display text
  `push store failed`. The private string is the driver's `Display`, which for database errors is
  PostgreSQL's message without the `DETAIL` line, so a CHECK violation's "Failing row contains"
  detail, which would include the token, is not captured.
- A failed `register` or `rotate` rolls back, so a rotation never loses the predecessor without
  registering the successor.
- A stored row that no longer parses (unknown platform, a token outside the rule) fails the list
  with `Internal` rather than being dropped.

## Privacy boundary

Device tokens are secrets-adjacent: anyone holding one can push to that device. `DeviceToken` has
no `Display` and a redacted `Debug`, and no Baukit error or log line carries one. `RegisteredDevice`
and `DeviceRegistration` derive `Debug` through the redacted field. `PushMessage` and
`PushOutcome` still carry the token as a plain `String` with a derived `Debug`; see open
decisions. The time zone and claim dates are personal data and are erased with the owner through
the cascading foreign keys or the erase functions. The advisory lock key is a hash of the owner
UUID and appears only in `pg_locks`.

## Supported runtimes

Tokio with SQLx 0.9 against PostgreSQL. The container tests run the `baukit-test` PostgreSQL
image. The port and the in-memory fakes have no runtime requirement.

## Breaks

- `baukit-push` gains `chrono` and `uuid` as required dependencies. No existing public item
  changed. The lib docs and README replace the hand-rolled pruning example with
  `invalidate_dead_tokens`.

No wire field was added or renamed. No version changed.

## Product code to remove on adoption

- Leitbild: `register_push_token`, `unregister_push_token`, and `push_tokens` in
  `crates/leitbild-postgres/src/reminder.rs:152-250`, the three port methods at
  `crates/leitbild-ports/src/lib.rs:276-286`, the in-memory copy in
  `crates/leitbild-bin/src/lib.rs` around line 1215, and the `push_tokens` table. The rejection
  branch in `crates/leitbild-notifications/src/lib.rs:47-58` stops failing the job for
  `DeviceNotRegistered` once the worker calls `invalidate_dead_tokens`.
- Eigenruhe: `upsert_device_token`, `delete_device_token`, `delete_dead_tokens`,
  `claim_notification`, and `release_notification` for the weekly kinds in
  `crates/eigenruhe-postgres/src/notifications.rs`, the matching port methods, the cap check and
  `row_cap_per_user` 422 for device tokens, and `device_push_tokens` plus `notification_log` rows
  for weekly kinds. `evaluation_owners` becomes a join on `push_devices`.
- Hebkit: `upsert_device_token`, `delete_device_token`, `evict_device_tokens`,
  `claim_notification_delivery`, and `release_notification_delivery` in
  `crates/hebkit-postgres/src/adapters/postgres/notifications.rs`, the five port methods in
  `crates/hebkit-ports/src/notification_repository.rs:17-53`, and the `device_push_tokens` and
  `push_notification_deliveries` tables.

## Product adoption follow-ups (deferred)

- Leitbild first: copy `0001_baukit_push_devices.sql`, add the owner foreign key to
  `user_identities`, drop `push_tokens` and the installation key, and have the worker call
  `invalidate_dead_tokens` after each send. Wire mobile registration, which has no caller today,
  with a stored last token and `rotate`.
- Eigenruhe: copy both migrations, join `push_devices` in `evaluation_owners`, switch weekly
  claims to `DeliveryClaimStore` with the ISO week's Monday as the local date, keep
  `connection_reattention` in its own table, and replace the register-then-unregister rotation
  with one `rotate` route.
- Hebkit: copy both migrations, use `DeliveryClaimStore` for `training_reminder`, and replace
  the register-then-unregister rotation with `rotate`.
- All three: schedule `purge_delivery_claims` where claims are used, and read the time zone from
  `RegisteredDevice::time_zone` where the product schedules by device.

## TypeScript follow-up

Eigenruhe's `PushRegistrationService` and Hebkit's are the same class: permission check, token
fetch, register, best-effort unregister of the stored predecessor, and secure-storage persistence.
Once products expose a rotate route, that class belongs next to `@baukit/notifications-expo` as a
registration helper that sends the stored predecessor with the new token. No TypeScript package
changed in this item.

## Open decisions

- `PushMessage::token` and `PushOutcome::token` are plain `String` fields with derived `Debug`, so
  a product that logs `?outcome` logs a token. Switching them to `DeviceToken` is a break across
  every sender and the Expo adapter, and it belongs in its own change.
- Receipts that are still `Accepted` after the send are never polled again, so a
  `DeviceNotRegistered` that Expo settles later is missed until the next send to that token. A
  deferred receipt poll would need persisted ticket IDs, which no product stores today.

## Product defects found

- Eigenruhe and Hebkit key tokens per owner. A failed unregister on sign-out, which both clients
  swallow, leaves the token under the old owner, so the next account on the device receives the
  previous account's notifications.
- Eigenruhe rejects the eleventh device, so a user at ten devices cannot rotate a token: the new
  registration is refused before the old one is removed.
- Leitbild never prunes `DeviceNotRegistered` tokens, and any rejection fails the reminder job
  permanently, so an owner with one dead device logs a failed job every day.
- Leitbild's mobile app never registers a push token; `registerPushToken` has no caller and the
  app does not depend on `expo-notifications`.
- Hebkit evicts dead tokens across all owners by token while its schema allows the same token under
  several owners, so one receipt deletes every owner's row for that token.
- Neither Eigenruhe nor Hebkit deletes old delivery claims.
