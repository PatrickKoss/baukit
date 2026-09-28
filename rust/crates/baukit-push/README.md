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

```rust,ignore
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
use chrono::Utc;

async fn deliver(
    sender: &impl PushSender,
    registry: &impl DeviceRegistry,
    messages: Vec<PushMessage>,
) -> Result<u64, Box<dyn std::error::Error>> {
    let sent_at = Utc::now();
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

## Daily delivery claims

A scheduled sender claims a `DeliveryClaim` keyed by owner, local date, and
`DeliveryKind` before it sends. `claim` returns `false` when another worker
already holds it. If the whole send fails, `release` the claim so a later run
can try again. Event-driven senders do not need claims, which is why this is a
separate `DeliveryClaimStore` port with its own table.

```rust,ignore
let claim = DeliveryClaim::new(owner_id, local_date, DeliveryKind::new("daily_reminder")?);
if !claims.claim(claim.clone(), Utc::now()).await? {
    return Ok(());
}
let sent_at = Utc::now();
match sender.send(messages).await {
    Ok(outcomes) => {
        registry.invalidate_dead_tokens(&outcomes, sent_at).await?;
    }
    Err(error) => {
        claims.release(claim).await?;
        return Err(error.into());
    }
}
```

A `DeliveryKind` is 1 to 64 bytes of `[a-z0-9_.-]` starting with a letter or
digit. With `sqlx-postgres`, `PostgresDeliveryClaimStore` uses the table in
`POSTGRES_PUSH_DELIVERY_CLAIMS_MIGRATION_SQL`. `erase_owner_delivery_claims`
removes one owner's claims, and `purge_delivery_claims` deletes one bounded
batch of claims for local dates before a cutoff; call it until it returns less
than the batch size.

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
use chrono::Utc;

async fn deliver(
    sender: &impl PushSender,
    registry: &impl DeviceRegistry,
    pending: &impl PendingReceiptStore,
    messages: Vec<PushMessage>,
) -> Result<(), Box<dyn std::error::Error>> {
    let sent_at = Utc::now();
    let outcomes = sender.send(messages).await?;
    registry.invalidate_dead_tokens(&outcomes, sent_at).await?;
    pending.record_accepted(&outcomes, sent_at).await?;
    Ok(())
}
```

`record_accepted` stores ticket ID, token, and `sent_at` for every `Accepted`
outcome, first due `RECEIPT_POLL_DELAY` (15 minutes) after the send.
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

```rust,ignore
use std::num::NonZeroU32;

use baukit_jobs::{ClaimedJob, JobCancellation, JobError, JobFuture, JobHandler};
use baukit_push::{
    ExpoPushSender, PendingReceiptStore, PostgresDeviceRegistry, PostgresPendingReceiptStore,
    RECEIPT_RETENTION, ReceiptPollError, poll_pending_receipts,
};
use chrono::Utc;

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
            let now = Utc::now();
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
`POSTGRES_PUSH_PENDING_RECEIPTS_MIGRATION_SQL`. The table has no owner column;
a row holds a token for at most the retention window until `purge` removes it.

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
retryable. Per-notification refusals are not errors at all; check
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
