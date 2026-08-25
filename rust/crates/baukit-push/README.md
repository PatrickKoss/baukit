# baukit-push

`baukit-push` delivers notifications to devices through a provider-neutral
`PushSender` port and ships one adapter for it, `ExpoPushSender`. Domain code
builds `PushMessage` values and reads `PushOutcome` values; nothing above the
port names Expo. Deciding *who* gets notified and *when* stays in the product.

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

`PushMessage` carries a token, a title, a body, an ordered `data` map delivered
with the notification, and an optional `channel_id` that Android reads to pick a
notification channel.

## Two-phase Expo delivery

Expo does not confirm delivery in the send response. `/push/send` answers with
one *ticket* per notification, meaning only that Expo accepted it. Delivery is
confirmed later through `/push/getReceipts`. `ExpoPushSender` runs both phases
per chunk, which collapses into three delivery states:

| `PushDeliveryStatus` | Meaning |
|---|---|
| `Delivered` | Expo handed the notification to APNs or FCM. |
| `Rejected(PushRejection)` | Expo refused it, at the ticket or receipt stage. |
| `Accepted` | Expo took it and has not settled a receipt yet. |

`Accepted` is neither success nor failure. The notification is in flight, so
never resend on it.

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

## Pruning dead tokens

A device token stops working once the app is uninstalled or the user turns
notifications off. Expo reports that as `DeviceNotRegistered`. Nothing prunes
those tokens for you, and the same failures repeat on every send until you do.
Delete them after each batch:

```rust
use baukit_push::{PushMessage, PushSender};

async fn deliver(
    sender: &impl PushSender,
    messages: Vec<PushMessage>,
) -> Result<Vec<String>, baukit_push::PushError> {
    let outcomes = sender.send(messages).await?;
    Ok(outcomes
        .iter()
        .filter(|outcome| outcome.is_token_dead())
        .map(|outcome| outcome.token.clone())
        .collect())
}
```

`is_token_dead` is true only for `DeviceNotRegistered`. Every other rejection
describes the notification, not the token, so deleting on `MessageTooBig` would
throw away a working device.

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
sender:

```toml
[dev-dependencies]
baukit-push = { workspace = true, features = ["test-support"] }
```

Every token delivers by default. Script exceptions per token with `reject` and
`accept_without_receipt`, or fail a whole batch with `fail_with`. Read back what
a service under test sent through `batches`, `messages`, `outcomes`, and
`dead_tokens`. Clones share one recording, so a clone handed to a service still
reports its sends.

The fake lives here rather than in `baukit-test` because it would otherwise pull
this opt-in crate into the dependencies of every product that uses the test kit.
