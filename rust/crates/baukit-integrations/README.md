# baukit-integrations

`baukit-integrations` contains two small provider ports: paged imports and
credential probes. It carries no HTTP client, database, or provider adapter, so
depending on it does not pull in a driver you never call.

The crate is opt-in. It is not part of the generated backend template and is not
wired into `baukit_config::BaukitConfig`.

## The port

```rust,ignore
pub trait IntegrationConnector: Send + Sync {
    type Record;

    fn provider_id(&self) -> &str;

    fn verify_webhook(
        &self,
        headers: &BTreeMap<String, String>,
        body: &[u8],
    ) -> Result<VerifiedWebhook, WebhookVerificationError>;

    fn requires_credentials(&self) -> bool { true }

    fn fetch_page<'a>(
        &'a self,
        job: &'a ClaimedConnectorJob,
        credentials: Option<&'a [u8]>,
    ) -> ConnectorFuture<'a, Self::Record>;
}
```

`Record` is your normalized type. The contract never looks inside it, which is
the point: `ConnectorPage<T>` carries the batch and the cursor, and you keep
your own shape.

Boxed futures rather than `async fn` keep the trait object safe, so a registry
can hold `Arc<dyn IntegrationConnector<Record = MyRecord>>` keyed by
`provider_id`. This matches `CredentialVault` and `PushSender`.

## One retry vocabulary

`ConnectorError` carries a `RetryClass` re-exported from `baukit-http`. There is
no second retry enum to translate into. Whatever
`classify_http_status` decided at the response boundary is what the runner sees:

```rust
use baukit_http::classify_transport_error;
use baukit_integrations::{ConnectorError, RetryClass};

fn on_transport_failure(is_timeout: bool) -> ConnectorError {
    let class = classify_transport_error(is_timeout);
    ConnectorError::new(
        if is_timeout { "timeout" } else { "provider_unavailable" },
        class,
    )
}

assert_eq!(on_transport_failure(true).class, RetryClass::Timeout);
assert!(on_transport_failure(false).is_retryable());
assert!(!ConnectorError::revoked("token_revoked").is_retryable());
```

The `code` is stable, non-localized diagnostic text safe to persist and to show
an operator. Never put provider body text there; it may carry credentials.

## Composing with baukit-jobs

`baukit-integrations` does not depend on `baukit-jobs` and does not replace it.
The runner still owns leases, attempt caps, and backoff. Your handler is the
three lines between them: a job payload names the connection, you call
`fetch_page`, and the `RetryClass` decides requeue against terminal failure.

```rust
use baukit_integrations::{ConnectorError, ConnectorPage, RetryClass};
use baukit_jobs::JobError;

fn to_job_error(error: &ConnectorError) -> JobError {
    match error.retry_after() {
        Some(delay) => JobError::retryable_after(error.code, delay),
        None if error.is_retryable() => JobError::retryable(error.code),
        None => JobError::permanent(error.code),
    }
}

// A rate limit that named a delay honors it instead of the runner's backoff.
let limited = ConnectorError::new("rate_limited", RetryClass::RetryAfter(
    std::time::Duration::from_secs(30),
));
assert!(to_job_error(&limited).is_retryable());

// A revoked credential stops retrying; only the owner reconnecting fixes it.
assert!(!to_job_error(&ConnectorError::revoked("token_revoked")).is_retryable());

// A page that returns a cursor means the handler should enqueue the next window.
let page = ConnectorPage::with_cursor(vec!["record"], serde_json::json!({"after": "abc"}));
assert!(page.has_more());
```

Two rules the runner does not enforce for you. Requeue the next window only when
`ConnectorPage::has_more()` is true, otherwise the import never ends. And keep
`max_attempts` on the enqueued job, because `RetryClass::is_retryable` says the
call *can* succeed, not that it should be tried forever.

`JobError::retryable_after` overrides the runner's exponential delay but not the
persisted `max_attempts` cap. A provider that keeps asking for a 30 second wait
still lands in `failed` with `JobFailureReason::AttemptsExhausted`.

## Webhooks

`verify_webhook` does two jobs and the order matters. Check the signature before
parsing the body, so a caller reaching `WebhookVerificationError::InvalidEvent`
has already proven the sender holds the shared secret. That distinction is worth
a separate metric label; the two failures mean different things during an
incident.

What comes back is a `VerifiedWebhook`: the provider's `delivery_id`, a
`signature_digest` as a fallback dedupe key for providers that reuse or omit
delivery identifiers, the resolved `connection_id`, and the job to enqueue.

The connector computes the dedupe identity. It does not decide whether that
identity was already seen, because the answer lives in your delivery table.
`WebhookIngestResult::Accepted` or `Duplicate` is your store's verdict.

## Connection health

`ConnectionHealth::after_failure(class)` turns a retry class into what the owner
should be told:

```rust
use baukit_integrations::{ConnectionHealth, RetryClass};

assert_eq!(
    ConnectionHealth::after_failure(RetryClass::Unavailable),
    ConnectionHealth::Degraded,
);
assert!(ConnectionHealth::after_failure(RetryClass::Revoked).needs_owner_action());
```

`ConnectionStatus` accumulates that over attempts. Call `succeeded` to clear the
failure state and `failed` to record a code and the next retry time. It
serializes as snake case, so it can go straight into a status response.

## Credential probes

Credential checks are not import jobs. Implement `CredentialProbe` on the
product adapter that calls the provider account endpoint:

```rust,ignore
use baukit_integrations::{CredentialProbe, CredentialProbeFuture};

struct ProductCredentialProbe;

impl CredentialProbe for ProductCredentialProbe {
    fn probe<'a>(&'a self, credential: &'a [u8]) -> CredentialProbeFuture<'a> {
        Box::pin(async move {
            // Build provider headers, call the product endpoint, enforce the
            // response limit, parse the provider body, and return Baukit types.
            todo!()
        })
    }
}
```

A successful check returns `CredentialProbeSuccess` with an
`ExternalAccountId`. Baukit does not parse that identifier. Its `Debug` output
is redacted, it has no `Display` implementation, and it is limited to 1,024
bytes. Call `as_str` only when passing it to product-owned persistence or an
account model.

Failures use `CredentialProbeError`: `Revoked`, `MissingScope`, `RateLimited`,
`Timeout`, `Unavailable`, or `InvalidData`. `health()` maps them to the existing
connection health states. A valid `Retry-After` value stays on `RateLimited`.
Use `baukit_http::retry_after_from_headers` to parse delta seconds and HTTP
dates without copying header logic.

Set a finite client timeout and stop response reads at
`MAX_CREDENTIAL_PROBE_RESPONSE_BYTES`. Reject a larger response as
`InvalidData`, even if its prefix contains valid JSON. Discard provider response
text after classification. `CredentialProbeError` contains no dynamic string,
so public errors and outcome logs cannot accidentally include a token or
provider body.

## What stays in your product

Per-provider persistence, token exchange and refresh, the provider-specific
token, request and error types, endpoints, headers, required scopes, response
parsing, account models, cursor formats, and dedupe heuristics. Deciding that
two providers reported the same measurement is a judgement about your data.

The reasoning is in
[ADR 0002](../../../docs/adr/0002-integration-connector-contract.md).

## Testing

`baukit_test::FakeConnector` scripts the six scenarios worth testing: healthy,
rate limited, unavailable, timeout, revoked, and exhausted. The four transient
ones fail once and then succeed, so a test can prove a handler actually retries;
`Exhausted` fails forever, which is how you reach the attempt cap on purpose.

```rust,ignore
use baukit_test::{FakeConnector, FakeConnectorScenario};

let connector = FakeConnector::new("strava", FakeConnectorScenario::RateLimited);
assert!(connector.fetch_page(&job, None).await.is_err()); // first attempt
assert!(connector.fetch_page(&job, None).await.is_ok());  // retry succeeds
```

For credential adapters, use
`baukit_test::check_credential_probe_conformance`. It starts a scripted loopback
HTTP server and runs healthy, revoked, missing-scope, rate-limit, timeout,
unavailable, malformed-data, and oversized-body cases. The product supplies the
raw responses, which keeps its scope headers and JSON shape out of Baukit.

## Migration

This release does not change `IntegrationConnector`, `ConnectorError`, or the
existing health types. Existing import adapters continue to compile. A product
with a local token-check trait can implement `CredentialProbe`, replace its
account string with `ExternalAccountId` at the boundary, and translate its six
local failures one at a time. Keep product error codes and connection-status
models by mapping `CredentialProbeError::code()` and `health()` in the service
layer.
