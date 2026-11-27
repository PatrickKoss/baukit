# baukit-push

`baukit-push` delivers notifications to devices through a provider-neutral
`PushSender` port and ships one adapter for it, `ExpoPushSender`. Domain code
builds `PushMessage` values and reads `PushOutcome` values; nothing above the
port names Expo. A `DeviceRegistry` port stores which tokens belong to which
owner, and a separate `DeliveryClaimStore` keeps scheduled pushes at most once
per owner, local date, and kind. Deciding *who* gets notified and *when*,
the copy, quiet hours, and the channel stay in the product.

The crate is opt-in. It is not part of the generated backend template and is not
wired into `baukit_config::BaukitConfig`.

## The port

```rust
# use baukit_push::{PushFuture, PushMessage};
pub trait PushSender: Send + Sync {
    fn send<'a>(&'a self, batch: Vec<PushMessage>) -> PushFuture<'a>;
}
```

One call takes a whole batch. The adapter splits it into provider-sized chunks
itself, so callers do not manage batching. Outcomes cover every message in the
batch but arrive in no guaranteed order; match them to messages by token.

`PushMessage` carries a `DeviceToken`, a title, a body, an ordered `data` map
delivered with the notification, and an optional `channel_id` that Android
reads to pick a notification channel. `PushOutcome` carries the same
`DeviceToken`, so logging `?outcome` or `?message` prints
`DeviceToken(<redacted>)` instead of the token.

## Two-phase Expo delivery

Expo does not confirm delivery in the send response. `/push/send` answers with
one *ticket* per notification, meaning only that Expo accepted it. Delivery is
confirmed later through `/push/getReceipts`. `ExpoPushSender` runs both phases
per chunk, which collapses into three delivery states:

| `PushDeliveryStatus` | Meaning |
|---|---|
| `Delivered` | Expo handed the notification to APNs or FCM. |
| `Rejected(PushRejection)` | Expo refused it, at the ticket or receipt stage. |
| `Accepted(PushTicketId)` | Expo took it and has not settled a receipt yet. |

`Accepted` is neither success nor failure. The notification is in flight, so
never resend on it. Its ticket ID is what a later receipt poll asks about; see
[Deferred receipts](#deferred-receipts).

## Rejection vocabulary

Expo's error codes map onto `PushRejection`:

| Expo code | `PushRejection` | Retryable |
|---|---|---|
| `DeviceNotRegistered` | `DeviceNotRegistered` | no |
| `MessageTooBig` | `MessageTooBig` | no |
| `MessageRateExceeded` | `MessageRateExceeded` | yes |
| `InvalidCredentials` | `InvalidCredentials` | no |
| `MismatchSenderId`, `ProviderError` | `ProviderError` | yes |
| anything else | `Other(code)` | no |

An unrecognized code keeps Expo's own string rather than being dropped, so a new
provider code shows up in logs instead of vanishing into a generic failure.

## Device registry

A device token stops working once the app is uninstalled or the user turns
notifications off. Expo reports that as `DeviceNotRegistered`, and the same
failure repeats on every send until the token is gone. The `DeviceRegistry`
port owns the tokens and removes dead ones in one call after each send:

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

`invalidate_dead_tokens` removes only tokens whose outcome
`is_token_dead`, which is true only for `DeviceNotRegistered`. Every other
rejection describes the notification, not the token, so deleting on
`MessageTooBig` would throw away a working device. A device registered again
after `sent_at` keeps its token, because Expo's verdict predates the new
registration. Outcomes without a dead token cost no store round trip.

| Method | Behavior |
|---|---|
| `register` | Inserts or refreshes the token for `registration.owner_id`. A token another owner holds moves to this owner. |
| `rotate` | Removes the owner's previous token and registers the new one in one step. A previous token the owner does not hold is ignored, so a retry is safe. |
| `unregister` | Removes one of the owner's tokens and returns whether it existed. |
| `list_for_owner` | Returns the owner's devices, most recently registered first. |
| `invalidate` | Removes the given tokens if they were last registered at or before `sent_at`. |
| `erase_owner` | Removes every device of the owner. |

Each owner keeps at most `DEFAULT_DEVICES_PER_OWNER` (10) devices unless the
store is built with `with_devices_per_owner`. A registration over the cap
evicts the owner's devices with the oldest `last_registered_at`, breaking ties
on `created_at` and then on the token, and reports how many went in
`RegistrationOutcome::evicted`. The device being registered is never evicted,
even when its instant is older than the others. Rotating removes the
predecessor first, so rotating at the cap evicts nothing.

`DeviceToken` accepts 1 to 512 bytes of visible ASCII. Its `Debug` output is
redacted and it has no `Display`; read it with `expose` only where it goes to
the provider or the store. `PushValidationError` and `PushStoreError` never
contain a token. `DeviceTimeZone` checks the shape of an IANA name only, so
resolve it with a time zone database before registering if the product
schedules by it. `DevicePlatform` is `Ios` or `Android`.

### PostgreSQL store

The `sqlx-postgres` feature adds `PostgresDeviceRegistry` over the table in
`POSTGRES_PUSH_DEVICES_MIGRATION_SQL`, which is available without the feature.
Copy the SQL into the product's migrations and add the owner foreign key the
header shows, with `ON DELETE CASCADE`. Nothing migrates on startup.

```toml
[dependencies]
baukit-push = { workspace = true, features = ["sqlx-postgres"] }
```

The token is the primary key. `register` and `rotate` run in one transaction
under a per-owner advisory lock, so concurrent registrations cannot overshoot
the cap. `erase_owner_push_devices` takes any `PgExecutor` and runs inside the
product's erasure transaction when the owner row stays or no foreign key
exists.

The reference migrations ship as constants. Apply them in this order:

| File | Constant | What it adds |
|---|---|---|
| `0001_baukit_push_devices.sql` | `POSTGRES_PUSH_DEVICES_MIGRATION_SQL` | the `push_devices` table |
| `0002_baukit_push_delivery_claims.sql` | `POSTGRES_PUSH_DELIVERY_CLAIMS_MIGRATION_SQL` | the `push_delivery_claims` table, see [daily delivery claims](#daily-delivery-claims) |
| `0003_baukit_push_pending_receipts.sql` | `POSTGRES_PUSH_PENDING_RECEIPTS_MIGRATION_SQL` | the pending receipt table, see [deferred receipts](#deferred-receipts) |
| `0004_baukit_push_pending_receipt_owners.sql` | `POSTGRES_PUSH_PENDING_RECEIPT_OWNERS_MIGRATION_SQL` | `owner_id` on pending receipts |
| `0005_baukit_push_install_secrets.sql` | `POSTGRES_PUSH_INSTALL_SECRETS_MIGRATION_SQL` | the nullable `push_devices.install_secret_hash` column and a `NOT VALID` check that it is 32 bytes |
| `0006_baukit_push_delivery_claim_ids.sql` | `POSTGRES_PUSH_DELIVERY_CLAIM_IDS_MIGRATION_SQL` | the nullable `push_delivery_claims.claim_id` column, so `release` deletes only the claim its token won |
| `0007_baukit_push_validate_install_secret_hash.sql` | `POSTGRES_PUSH_VALIDATE_INSTALL_SECRET_HASH_MIGRATION_SQL` | validates the 0005 check in its own transaction, without blocking writes |

None of 0005 to 0007 rewrites a table. A device row with a `NULL` install
secret hash still moves to a new owner without a secret until the
installation registers again with one. A claim made before 0006 has no
`claim_id`, so `release` never deletes it and `purge` removes it later.

## Daily delivery claims

A scheduled sender claims a `DeliveryClaim` keyed by owner, local date, and
`DeliveryKind` before it sends. `claim` returns `None` when another worker
already holds it, and a `ClaimToken` otherwise. If the send fails before any
notification went out, `release` the claim with that token so a later run can try again. A `PushError::Partial` means earlier
chunks already went out: record its `outcomes` and keep the claim, because a
retry would send those notifications twice. Event-driven senders do not need claims, which is why this is a
separate `DeliveryClaimStore` port with its own table.

```rust
# use baukit_push::{
#     DeliveryClaim, DeliveryClaimStore, DeliveryKind, DeviceRegistry, PushMessage, PushSender,
# };
# use jiff::{Timestamp, civil::Date};
# use uuid::Uuid;
# async fn example(
#     claims: &impl DeliveryClaimStore,
#     registry: &impl DeviceRegistry,
#     sender: &impl PushSender,
#     owner_id: Uuid,
#     local_date: Date,
#     messages: Vec<PushMessage>,
# ) -> Result<(), Box<dyn std::error::Error>> {
let claim = DeliveryClaim::new(owner_id, local_date, DeliveryKind::new("daily_reminder")?);
let Some(token) = claims.claim(claim.clone(), Timestamp::now()).await? else {
    return Ok(());
};
let sent_at = Timestamp::now();
match sender.send(messages).await {
    Ok(outcomes) => {
        registry.invalidate_dead_tokens(&outcomes, sent_at).await?;
    }
    Err(error) => {
        if error.completed_outcomes().is_empty() {
            claims.release(claim, token).await?;
        } else {
            registry.invalidate_dead_tokens(error.completed_outcomes(), sent_at).await?;
        }
        return Err(error.into());
    }
}
# Ok(())
# }
```

A `DeliveryKind` is 1 to 64 bytes of `[a-z0-9_.-]` starting with a letter or
digit. With `sqlx-postgres`, `PostgresDeliveryClaimStore` uses the table in
`POSTGRES_PUSH_DELIVERY_CLAIMS_MIGRATION_SQL`. `erase_owner_delivery_claims`
removes one owner's claims.

Claims grow by one row per owner, kind, and day. `DeliveryClaimStore::purge`
deletes one bounded batch of claims for local dates before a cutoff, oldest
date first; call it until it returns less than the batch size. The memory
store purges the same way, so a service test can check its purge job. To purge
inside your own transaction, call `purge_delivery_claims` with the executor.

## Deferred receipts

`ExpoPushSender` asks for receipts right after the send, and Expo often has
not settled them yet. Those notifications come back as `Accepted(ticket)`. A
`DeviceNotRegistered` that Expo settles later would otherwise go unnoticed
until the next send to that token. Expo recommends checking receipts 15
minutes after the send, keeps them for 24 hours, and takes at most 1000
ticket IDs per `getReceipts` request (`MAX_RECEIPT_BATCH_SIZE`; the adapter
splits larger lists itself).

Record the accepted tickets after each send:

```rust
use baukit_push::{DeviceRegistry, PendingReceiptStore, PushMessage, PushSender};
use jiff::Timestamp;
use uuid::Uuid;

async fn deliver(
    sender: &impl PushSender,
    registry: &impl DeviceRegistry,
    pending: &impl PendingReceiptStore,
    owner_id: Uuid,
    messages: Vec<PushMessage>,
) -> Result<(), Box<dyn std::error::Error>> {
    let sent_at = Timestamp::now();
    let outcomes = sender.send(messages).await?;
    registry.invalidate_dead_tokens(&outcomes, sent_at).await?;
    pending.record_accepted(owner_id, &outcomes, sent_at).await?;
    Ok(())
}
```

`record_accepted` stores ticket ID, owner, token, and `sent_at` for every
`Accepted` outcome of one owner's send, first due `RECEIPT_POLL_DELAY` (15
minutes) after the send. A product that sends to several owners in one batch
calls `accepted_receipts` per owner and passes the result to `record`.
`poll_pending_receipts` then does one bounded run:

1. `take_due` takes up to `limit` due tickets and makes them due again one
   `RECEIPT_POLL_DELAY` later, so concurrent pollers take disjoint batches and
   an unsettled ticket goes to the back of the queue.
2. It reads their receipts through the `PushReceiptSource` port, which
   `ExpoPushSender` implements.
3. It passes each `DeviceNotRegistered` token to `DeviceRegistry::invalidate`
   with the `sent_at` of its own send, so a device registered again after that
   send keeps its token.
4. It deletes the settled tickets. Tickets without a receipt stay for a later
   run.

The returned `ReceiptPoll` counts the tickets taken and settled and the tokens
invalidated. Fewer taken than `limit` means nothing else is due. A failure is a
`ReceiptPollError`; the taken tickets stay recorded and come due again.
`purge` deletes tickets older than `RECEIPT_RETENTION` (24 hours), whose
receipts Expo no longer has.

Baukit ships no scheduler. Drive the poll from the product's job runner, for
example a `baukit-jobs` handler that a `FixedUtcInterval` re-enqueues every few
minutes:

```rust
use std::num::NonZeroU32;

use baukit_jobs::{ClaimedJob, JobCancellation, JobError, JobFuture, JobHandler};
use baukit_push::{
    ExpoPushSender, PendingReceiptStore, PostgresDeviceRegistry, PostgresPendingReceiptStore,
    RECEIPT_RETENTION, ReceiptPollError, poll_pending_receipts,
};
use jiff::Timestamp;

const RECEIPT_BATCH: NonZeroU32 = NonZeroU32::new(1000).expect("not zero");

struct PollPushReceipts {
    sender: ExpoPushSender,
    pending: PostgresPendingReceiptStore,
    registry: PostgresDeviceRegistry,
}

impl JobHandler for PollPushReceipts {
    fn job_types(&self) -> &'static [&'static str] {
        &["push_receipts.poll"]
    }

    fn handle<'a>(
        &'a self,
        _job: &'a ClaimedJob,
        _cancellation: JobCancellation,
    ) -> JobFuture<'a, Result<(), JobError>> {
        Box::pin(async move {
            let now = Timestamp::now();
            self.pending
                .purge(now - RECEIPT_RETENTION, RECEIPT_BATCH)
                .await
                .map_err(|error| JobError::retryable(error.to_string()))?;
            loop {
                let poll = poll_pending_receipts(
                    &self.sender,
                    &self.pending,
                    &self.registry,
                    now,
                    RECEIPT_BATCH,
                )
                .await
                .map_err(job_error)?;
                if poll.taken < u64::from(RECEIPT_BATCH.get()) {
                    return Ok(());
                }
            }
        })
    }
}

fn job_error(error: ReceiptPollError) -> JobError {
    match &error {
        ReceiptPollError::Push(push) => match push.retry_after() {
            Some(delay) => JobError::retryable_after(error.to_string(), delay),
            None => JobError::retryable(error.to_string()),
        },
        ReceiptPollError::Store(_) => JobError::retryable(error.to_string()),
    }
}
```

The loop ends because every taken ticket moves past `now`. With
`sqlx-postgres`, `PostgresPendingReceiptStore` uses the table in
`POSTGRES_PUSH_PENDING_RECEIPTS_MIGRATION_SQL` followed by
`POSTGRES_PUSH_PENDING_RECEIPT_OWNERS_MIGRATION_SQL`, which adds the
`owner_id` column. Add the owner foreign key the second file's header shows,
with `ON DELETE CASCADE`, so erasing an owner deletes the device tokens their
pending tickets hold instead of leaving them until `purge`. Without the
foreign key, call `PendingReceiptStore::erase_owner`, or
`erase_owner_pending_receipts` with the product's erasure transaction.

## Guarded transport

Every ticket and receipt request goes through a `baukit-egress`
`GuardedClient`. By default it reaches only the scheme, host and port of the
configured endpoint, with the push request timeout. Replace it with
`ExpoPushSender::with_guarded_client` to use another policy:

```rust,no_run
use baukit_egress::{EgressOptions, GuardedClient};
use baukit_push::{ExpoPushSender, PushOptions};

# fn build() -> Result<ExpoPushSender, Box<dyn std::error::Error>> {
let options = PushOptions::default();
let egress = EgressOptions::default()
    .with_request_timeout(options.request_timeout())?;
let sender = ExpoPushSender::with_options(options)?
    .with_guarded_client(GuardedClient::new(egress)?);
# Ok(sender)
# }
```

The supplied client's policy, timeouts, response limits and retry classification
apply to every provider request. Its defaults require public HTTPS. Set the
egress request timeout explicitly to use the push timeout, as above.
Guarded errors omit provider bodies. The sender's `Debug` output omits
credentials and endpoint URLs. There is no unguarded transport and no extra
feature flag.

If an immediate receipt fetch fails after Expo accepts a ticket, `send` returns
`PushDeliveryStatus::Accepted(ticket)` for it. Store these tickets with
`PendingReceiptStore::record_accepted` and poll them later. Retrying the send
would duplicate notifications. Explicit `PushReceiptSource::receipts` calls
still return receipt-fetch failures.

## Retries

A failure that stops the whole request is a `PushError::Transport` carrying a
`baukit_http::RetryClass`, the same classification the other outbound clients
use. An Expo rate limit that names a `Retry-After` reaches the caller as a
concrete delay:

```rust
# use std::time::Duration;
# fn example(error: &baukit_push::PushError) {
if error.is_retryable() {
    let delay = error.retry_after().unwrap_or(Duration::from_secs(30));
    // schedule the batch again after `delay`
}
# }
```

A malformed provider response is a `PushError::InvalidResponse` and is never
retryable. When a later chunk of a multi-chunk batch fails, `send` returns
`PushError::Partial` with the outcomes of the chunks Expo already handled and
the error that stopped the rest; `is_retryable` and `retry_after` follow that
inner error. Retry only the messages without an outcome. Per-notification refusals are not errors at all; check
`PushRejection::is_retryable` on each outcome instead.

## Configuration

`PushConfig` is a `Deserialize` + `baukit_config::Validate` section a product
embeds in its own product config, which puts environment overrides on the usual
nested path (`ORDERS__PUSH__BATCH_SIZE`).

| Field | Default | Notes |
|---|---|---|
| `endpoint` | `https://exp.host/--/api/v2/push/send` | Full send endpoint. The receipt URL is derived by replacing `send` with `getReceipts`. |
| `access_token` | empty | `Secret<String>`; empty means no `Authorization` header. |
| `batch_size` | `100` | 1 to 100, Expo's per-request limit. |
| `request_timeout_ms` | `8000` | Applied to the ticket and receipt requests separately. |

`PushOptions::from_config` validates the whole section at once and reports every
problem together, so a bad base URL or batch size fails at startup instead of on
the first notification. In code, build options directly:

```rust
use std::time::Duration;

use baukit_push::{ExpoPushSender, PushOptions};

# fn build() -> Result<ExpoPushSender, baukit_push::PushOptionsError> {
let options = PushOptions::default()
    .with_batch_size(50)?
    .with_request_timeout(Duration::from_secs(5))?;
ExpoPushSender::with_options(options)
# }
```

Pass the full send URL to `PushOptions::new`, including `/push/send`. This also
works for local mock servers and proxies with a different path prefix.

Expo only requires an access token when the project enforces push security. Set
one with `with_access_token` and it is sent as a bearer header; `Debug` output
redacts it.

## Testing

Enable the `test-support` feature for `FakePushSender`, an in-memory recording
sender and receipt source, and for `MemoryDeviceRegistry`,
`MemoryDeliveryClaimStore`, and `MemoryPendingReceiptStore`, which follow the
PostgreSQL adapters' rules:

```toml
[dev-dependencies]
baukit-push = { workspace = true, features = ["test-support"] }
```

Every token delivers by default. Script exceptions per token with `reject` and
`accept_without_receipt`, settle an accepted token's later receipt with
`settle_receipt`, or fail every send and receipt request with `fail_with`. Read
back what a service under test sent through `batches`, `messages`, `outcomes`,
`dead_tokens`, and `receipt_requests`. Clones share one recording, so a clone
handed to a service still reports its sends.

The fake lives here rather than in `baukit-test` because it would otherwise pull
this opt-in crate into the dependencies of every product that uses the test kit.
