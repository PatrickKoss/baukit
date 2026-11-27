# Send push and local notifications

At the end of this page your backend stores device tokens from an Expo app, sends pushes
through Expo, removes dead tokens and polls late receipts. Your app also plans local
reminders without touching notifications that other code owns. The page is for product
developers who add notifications to a generated product.

Push is opt-in. No template generates it, and `baukit_config::BaukitConfig` has no push
section. You add `baukit-push` to the backend and the notification packages to the mobile
app yourself.

## The pieces

| Part | Where it runs | Job |
|---|---|---|
| `baukit-push` | backend | `PushSender` port, `ExpoPushSender` adapter, `DeviceRegistry`, `DeliveryClaimStore`, `PendingReceiptStore` |
| `@baukit/notifications-core` | app | plans local notifications and computes what to keep, cancel and schedule |
| `@baukit/notifications-expo` | app | runs that plan against `expo-notifications` |

Remote push goes from the backend through Expo to APNs or FCM. Local notifications never
leave the device. A product can use either one alone.

## Add the crate

```toml
[dependencies]
baukit-push = { workspace = true, features = ["sqlx-postgres"] }

[dev-dependencies]
baukit-push = { workspace = true, features = ["test-support"] }
```

`sqlx-postgres` adds `PostgresDeviceRegistry`, `PostgresDeliveryClaimStore` and
`PostgresPendingReceiptStore`. `test-support` adds `FakePushSender` and the memory stores.
Both feature names come from `rust/crates/baukit-push/README.md`.

## Copy the migrations

Nothing migrates on startup. Copy these files from `rust/crates/baukit-push/migrations/`
into the product's ordered migrations, in this order:

| File | Constant | What it adds |
|---|---|---|
| `0001_baukit_push_devices.sql` | `POSTGRES_PUSH_DEVICES_MIGRATION_SQL` | `push_devices`, keyed by token |
| `0002_baukit_push_delivery_claims.sql` | `POSTGRES_PUSH_DELIVERY_CLAIMS_MIGRATION_SQL` | `push_delivery_claims`, only for scheduled pushes |
| `0003_baukit_push_pending_receipts.sql` | `POSTGRES_PUSH_PENDING_RECEIPTS_MIGRATION_SQL` | `push_pending_receipts`, only for receipt polling |
| `0004_baukit_push_pending_receipt_owners.sql` | `POSTGRES_PUSH_PENDING_RECEIPT_OWNERS_MIGRATION_SQL` | `owner_id` on pending receipts |
| `0005_baukit_push_install_secrets.sql` | `POSTGRES_PUSH_INSTALL_SECRETS_MIGRATION_SQL` | `install_secret_hash` on devices, check added `NOT VALID` |
| `0006_baukit_push_delivery_claim_ids.sql` | `POSTGRES_PUSH_DELIVERY_CLAIM_IDS_MIGRATION_SQL` | `claim_id` on delivery claims |
| `0007_baukit_push_validate_install_secret_hash.sql` | `POSTGRES_PUSH_VALIDATE_INSTALL_SECRET_HASH_MIGRATION_SQL` | validates the 0005 check in its own transaction |

The constants live in `rust/crates/baukit-push/src/lib.rs`. `0004` runs
`DELETE FROM push_pending_receipts` before it adds the column, so tickets recorded earlier
are dropped.

baukit does not know your owner table. Each file header shows the foreign key to add in a
product migration, with `ON DELETE CASCADE`, for `push_devices`, `push_delivery_claims` and
`push_pending_receipts`. With those keys, deleting an owner deletes their tokens. In
`rust/migrations.toml`, `baukit-push` has no `after` entry, so its files do not depend on
another crate's tables. [Order SQL migrations](../migrations/authoring-sql-migrations.md)
covers the rules for writing your own.

## Register devices from the app

baukit ships no client helper for push registration. The app gets its Expo push token with
`expo-notifications` and posts it to a product route with three more values:

- the platform, `ios` or `android`
- an IANA time zone name, if the product schedules by local time
- an install secret

The install secret is a random string of 16 to 256 visible ASCII characters
(`MIN_INSTALL_SECRET_LENGTH`, `MAX_INSTALL_SECRET_LENGTH`). The app generates it once per
install, keeps it in secure storage and sends it with every registration. The registry
stores only its SHA-256 digest. A token moves to another owner only when that owner sends
the same secret, so one account cannot take over another account's device by replaying its
token.

The product route builds a `DeviceRegistration` and calls `DeviceRegistry::register`:

- `DeviceToken::new` accepts 1 to 512 bytes of visible ASCII. Its `Debug` output is
  redacted and it has no `Display`.
- `DevicePlatform::parse` maps `ios` and `android`.
- `DeviceTimeZone::new` checks the shape of the name only. Resolve it with a time zone
  database first if you schedule by it.
- `DeviceRegistration::new(owner_id, token, platform, registered_at)`, then
  `with_install_secret` and `with_time_zone`.

Map `PushStoreError::TokenHeldByAnotherOwner` to a `409`. Use `rotate` when Expo hands the
app a new token, and `unregister` on sign-out. This step needs a tested snippet. Read
`rust/crates/baukit-push/src/registry.rs` for the signatures. The route takes an owner from
the authenticated principal, so it follows the usual rules: the rate limiter wraps it and a
test shows a second principal cannot touch the first principal's devices.

Each owner keeps at most `DEFAULT_DEVICES_PER_OWNER` (10) devices. A registration over the
cap evicts the oldest ones and reports the count in `RegistrationOutcome::evicted`. Build
the store with `with_devices_per_owner` to change the cap.

## Send and remove dead tokens

`PushSender::send` takes a whole batch of `PushMessage` values. `ExpoPushSender` splits it
into chunks of `batch_size`, sends them and asks for receipts right away. After each send,
pass the outcomes to the registry. From `rust/crates/baukit-push/README.md` (doctest):

```rust
use baukit_push::{DeviceRegistry, PushMessage, PushSender};
use jiff::Timestamp;

async fn deliver(
    sender: &impl PushSender,
    registry: &impl DeviceRegistry,
    messages: Vec<PushMessage>,
) -> Result<u64, Box<dyn std::error::Error>> {
    let sent_at = Timestamp::now();
    let outcomes = sender.send(messages).await?;
    Ok(registry.invalidate_dead_tokens(&outcomes, sent_at).await?)
}
```

`invalidate_dead_tokens` removes a token only on `DeviceNotRegistered`. A device that
registered again after `sent_at` keeps its token.

Each `PushOutcome` has a `PushDeliveryStatus`:

- `Delivered`: Expo handed it to APNs or FCM.
- `Rejected(PushRejection)`: Expo refused it. `PushRejection::is_retryable` says whether a
  later send can work.
- `Accepted(PushTicketId)`: Expo took it and has no receipt yet. Never resend on it.

A failure of the whole request is a `PushError`. `is_retryable` and `retry_after` tell a
job when to try again. `PushError::Partial` means earlier chunks already went out. Its
`completed_outcomes()` lists them, so resend only the messages without an outcome.

All Expo requests go through a `baukit-egress` `GuardedClient`, limited to the configured
endpoint's scheme, host and port. [Call other services safely](outbound-and-limits.md)
covers egress.

## Configure the sender

Embed `PushConfig` in your product config. Environment overrides follow the nested path,
for example `ORDERS__PUSH__BATCH_SIZE`. Fields and defaults from
`rust/crates/baukit-push/src/config.rs`:

| Field | Default | Notes |
|---|---|---|
| `endpoint` | `https://exp.host/--/api/v2/push/send` | The receipt URL swaps `send` for `getReceipts` |
| `access_token` | empty | `Secret<String>`. Empty sends no `Authorization` header |
| `batch_size` | `100` | 1 to 100 |
| `request_timeout_ms` | `8000` | per ticket request and per receipt request |

`PushOptions::from_config` validates the section and reports every problem at startup.

## Poll late receipts

Expo often has no receipt right after the send. A `DeviceNotRegistered` that Expo settles
later would otherwise wait for the next send. Record the accepted tickets with
`PendingReceiptStore::record_accepted(owner_id, &outcomes, sent_at)`. They come due after
`RECEIPT_POLL_DELAY` (15 minutes).

baukit ships no scheduler. Run `poll_pending_receipts` from a `baukit-jobs` handler that a
`FixedUtcInterval` re-enqueues every few minutes. Each run takes up to `limit` due tickets,
reads their receipts, invalidates dead tokens and deletes settled tickets. Loop while
`ReceiptPoll::taken` equals the limit. Call `purge` with `now - RECEIPT_RETENTION` (24
hours) in the same job. The full handler is in the "Deferred receipts" section of
`rust/crates/baukit-push/README.md`. [Run background jobs](jobs.md) covers the worker.

## Send scheduled pushes once per day

A daily reminder must not go out twice when two workers run. Claim a `DeliveryClaim` keyed
by owner, local date and `DeliveryKind` before sending. `claim` returns `None` when another
worker holds it and a `ClaimToken` otherwise. If the send fails before anything went out,
`release` the claim with that token. On `PushError::Partial`, keep the claim. Call
`DeliveryClaimStore::purge` from a recurring job, because claims grow by one row per owner,
kind and day. Event-driven pushes need no claims.

## Erase an owner's push data

With the foreign keys from the migrations, erasing the owner row cascades. Without them,
call these inside the product's erasure transaction:

- `erase_owner_push_devices`
- `erase_owner_delivery_claims`
- `erase_owner_pending_receipts`

On the client, unregister the push token as a pre-server hook of `eraseProductProfile`.
[Erase user data and answer data subject requests](privacy.md) covers the rest.

## Plan local notifications

`@baukit/notifications-core` turns product occurrences into instants and a replacement
plan. The product decides which occurrences exist and what they say. From
`typescript/packages/notifications-core/README.md`:

```ts
import {
  planNotificationReplacement,
  resolveNotificationOccurrences,
} from '@baukit/notifications-core';

const resolution = resolveNotificationOccurrences({
  occurrences: [
    { logicalId: 'plan:2026-03-29', civilDate: '2026-03-29', civilTime: '02:30', contentDigest: 'v3' },
  ],
  timeZone: 'Europe/Berlin',
  gap: 'shiftForward',
  fold: 'earlier',
  clock: () => Date.now(),
  horizonDays: 14,
});
const plan = planNotificationReplacement(current, resolution.desired);
```

You pick the DST `gap` and `fold` policies. Neither has a default. Occurrences in the past,
outside the horizon or in a rejected gap land in `resolution.skipped` with a reason. An
entry stays scheduled only when its logical ID, instant and content digest all match, so
change the digest when the copy changes.

## Schedule them with Expo

`@baukit/notifications-expo` implements the platform port on `expo-notifications`. From
`typescript/packages/notifications-expo/README.md`:

```ts
import {
  createExpoOwnedNotificationScheduler,
  IOS_PENDING_NOTIFICATION_LIMIT,
} from '@baukit/notifications-expo';
import * as Notifications from 'expo-notifications';

const scheduler = createExpoOwnedNotificationScheduler(Notifications, {
  pendingLimit: IOS_PENDING_NOTIFICATION_LIMIT,
});
const outcome = await scheduler.replaceOwned(
  { namespace: 'reminders' },
  desired.map((entry) => ({
    ...entry,
    content: { content: { title: entry.title, body: entry.body }, channelId: 'reminders' },
  })),
);
```

Create one scheduler per app. It touches only requests with the identifier
`baukit:<namespace>:<logicalId>` and its marker under `baukitNotification` in
`content.data`. It never cancels everything. `IOS_PENDING_NOTIFICATION_LIMIT` is 64 and
counts every pending request on the device, so pass less when other features schedule too.
The package does not request permission, create Android channels or support repeating
triggers. Plan a rolling horizon and call `replaceOwned` again when the app opens or the
plan changes. `outcome.status` is `complete`, `incomplete` or `superseded`.

## Test it

On the backend, `FakePushSender` records batches and scripts rejections with `reject`,
`accept_without_receipt`, `settle_receipt` and `fail_with`. `MemoryDeviceRegistry`,
`MemoryDeliveryClaimStore` and `MemoryPendingReceiptStore` follow the PostgreSQL rules.

In the app, run `describeOwnedNotificationSchedulerContract` from
`@baukit/notifications-core/vitest` against your platform. `InMemoryNotificationPlatform`
is the reference platform. baukit runs the same checks inside Hermes on an Android emulator
with `make expo-notifications-conformance`. See
`examples/expo-notifications-conformance/README.md`.

## Reference

- [baukit-push README](../../rust/crates/baukit-push/README.md)
- [`@baukit/notifications-core` README](../../typescript/packages/notifications-core/README.md)
- [`@baukit/notifications-expo` README](../../typescript/packages/notifications-expo/README.md)
- [Push install secrets migration guide](../migrations/push-install-secrets.md)
- [Order SQL migrations](../migrations/authoring-sql-migrations.md)
- [ADR 0032: push delivery](../adr/0032-push-delivery.md)
- [ADR 0016: outbound HTTP through egress](../adr/0016-outbound-http-through-egress.md)
- [Expo notifications conformance example](../../examples/expo-notifications-conformance/README.md)
- [Localization contract](../platform/localization-contract.md)
