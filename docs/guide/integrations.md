# Connect a third-party provider

At the end of this page your backend imports records from a provider through a connector, accepts the provider's signed webhooks, links a user's provider account through an OAuth flow bound to their browser, and keeps the provider tokens encrypted at rest. The page is for product developers adding a provider integration. No template wires integrations, so you build each piece in your own backend from `baukit-integrations`, `baukit-credential-vault`, `baukit-jobs` and `baukit-test`.

The running example is a fictional provider with the id `example`. It has an OAuth authorize endpoint, a paged records API and HMAC-signed webhooks.

## What baukit gives you and what you write

Baukit ships the ports and the parts that are easy to get wrong:

- `IntegrationConnector`, the port your provider adapter implements, in `baukit-integrations`.
- `ConnectorError` with the shared `RetryClass` from `baukit-http`.
- `ConnectionHealth` and `ConnectionStatus` for what the owner sees.
- `CredentialProbe` for checking a stored credential.
- `OAuthStateService` behind the `oauth-state` feature, with PostgreSQL and axum adapters.
- `CredentialCipher` and the `CredentialVault` port in `baukit-credential-vault`.
- `FakeConnector`, `check_credential_probe_conformance` and `check_postgres_inbox_conformance` in `baukit-test`.

Your product keeps the rest. That is token exchange and refresh, provider request and response types, endpoints, scopes, cursor formats, account models, the vault table and the webhook delivery table. The split is recorded in [ADR 0002](../adr/0002-integration-connector-contract.md).

Add the crates to the adapter crate of your backend:

```toml
baukit-integrations = { version = "...", features = ["postgres", "axum"] }
baukit-credential-vault = "..."
```

The `postgres` feature turns on `oauth-state`. Use the version your other baukit crates pin.

## Step 1: store credentials in the vault

Do this first, because every later step reads a token. `CredentialVaultConfig` reads a keyring of `version:base64key` entries. The highest version is the write key. An empty keyring disables the vault and `cipher()` returns `None`.

Nest the config in your product settings. This snippet is a doctest in `rust/crates/baukit-credential-vault/README.md`:

```rust
use baukit_config::{Validate, ValidationErrors};
use baukit_credential_vault::CredentialVaultConfig;
use serde::Deserialize;

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct ProductConfig {
    credential_vault: CredentialVaultConfig,
}

impl Validate for ProductConfig {
    fn validate(&self) -> Result<(), ValidationErrors> {
        self.credential_vault.validate()
    }
}
```

Generate a key with `openssl rand -base64 32` and set it through your app prefix, for example `ORDERS__CREDENTIAL_VAULT__KEYRING=1:<key>`. A malformed ring fails validation at startup.

Then implement `CredentialVault` over your own table. `store` and `delete` take `owner_id` so the SQL can check ownership in the same statement. Use `seal_record` and `open_record` when an access token and refresh token must always open together, and store the returned `record_id` next to the fields. The crate README has the port skeleton and the rotation steps. Never derive `Debug` on a type that holds a decrypted field. `CredentialSecrets` implements neither `Debug` nor `Display`.

## Step 2: link the account with OAuth state

The user clicks "Connect" in your app. Your backend redirects them to the provider, and the provider calls you back with a `code` and the `state` you sent.

A `state` that carries the signed user id is open to login CSRF. An attacker starts a flow, sends the authorize URL to a victim, and the victim's provider account gets linked to the attacker's user. `OAuthStateService` closes that hole by tying the flow to the browser that started it.

Here is what the code does, checked against `rust/crates/baukit-integrations/src/oauth_state/`:

1. `start(StartOAuthFlow { user, provider, redirect, pkce })` creates three random 256-bit tokens. They are the `state`, a `BrowserBinding` and, with `Pkce::S256`, a PKCE verifier.
2. The store keeps the SHA-256 hash of the state and of the binding, never the raw values. The user id, provider, redirect and verifier sit in the same row. The row expires after `DEFAULT_OAUTH_STATE_TTL`, which is 600 seconds. `with_ttl` changes it.
3. Your start route sends the binding to the browser in the `__Host-baukit-oauth` cookie. `binding_set_cookie` builds it with `Max-Age` set to the TTL, `Path=/`, `Secure`, `HttpOnly` and `SameSite=Lax`. Lax is required because the provider's redirect back to you is a top-level cross-site navigation.
4. On the callback, `binding_from_headers` reads the cookie and `complete(&state, &binding)` deletes the row by state hash. The PostgreSQL store does that delete and writes a used marker in one statement, so exactly one concurrent callback wins.
5. Only after the delete does `complete` check expiry and then compare the binding hash in constant time. A wrong cookie returns `BrowserMismatch` and the state is gone. A retry with the right cookie then gets `AlreadyUsed`.

So the binding is a per-flow random cookie. It is not tied to your session cookie or to the logged-in user on the callback. The callback route carries no bearer, and `complete` returns the user who started the flow. Take the user from that result and never from the query string.

The test `another_browser_is_rejected_and_burns_the_state` in `rust/crates/baukit-integrations/src/oauth_state/tests.rs` shows the attack and the burn:

```rust
let attacker = service.start(input(Pkce::S256)).await.expect("start");
let victim = service.start(input(Pkce::S256)).await.expect("start");
let error = service
    .complete(&attacker.state, &victim.binding)
    .await
    .expect_err("mismatch");
assert!(matches!(error, OAuthStateError::BrowserMismatch));
let retry = service
    .complete(&attacker.state, &attacker.binding)
    .await
    .expect_err("burned");
assert!(matches!(retry, OAuthStateError::AlreadyUsed));
```

Wire it into your backend like this:

- Copy `OAUTH_STATE_MIGRATION_SQL` from `baukit_integrations::oauth_state::postgres` into a product migration. It creates `oauth_states` and `oauth_states_used`.
- Build `OAuthStateService::new(Arc::new(PostgresOAuthStateStore::new(pool)))`.
- Mount the start route behind your authentication, for example `GET /me/integrations/{provider}/start`. Respond 303 to the provider authorize URL with `started.state.expose()` and `started.pkce_challenge` in it, and set the cookie.
- Mount the callback outside authentication and inside the rate limiter. Parse the state with `OAuthState::parse`, call `complete`, check that `flow.provider` matches the path, exchange the code with `flow.pkce_verifier`, store the tokens through your vault, and clear the cookie with `binding_clear_cookie()`.
- Call `OAuthStateStore::purge_expired(now)` from a daily job.
- Map every `OAuthStateError` on the callback to one response. Log the variant, but do not tell the browser which one fired.

A browser holds one binding cookie. A second flow started in another tab replaces it, so the first tab's callback fails with `BrowserMismatch`.

The route wiring has no tested snippet. The crate README shows it as an untested `rust,ignore` block.

On the client, `@baukit/integrations-client` drives the redirect. `OAuthSessionCoordinator` saves a nonce before it opens the authorize URL and validates the return URL against `createReturnUrlValidator({ origin, paths })`. It returns `success`, `cancelled`, `timed_out`, `invalid_return` or `storage_mismatch`. That nonce check is a client-side guard. The server-side cookie check above is what stops login CSRF.

## Step 3: implement the connector

`IntegrationConnector` has an associated `Record` type, which is your normalized record. Implement `provider_id`, `verify_webhook` and `fetch_page`. `fetch_page` takes the claimed job and the decrypted credentials, and returns a `ConnectorPage<Record>` with the batch and an optional cursor. Override `requires_credentials` to return `false` for a provider that needs no token.

The trait returns boxed futures, so a registry can hold `Arc<dyn IntegrationConnector<Record = MyRecord>>` keyed by provider id.

Translate every provider failure into a `ConnectorError` with a `RetryClass`. Use `baukit_http::classify_http_status` for responses and `classify_transport_error` for transport errors. This is a doctest in `rust/crates/baukit-integrations/README.md`:

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

The `code` is a stable diagnostic string. Never copy provider body text into it, because it may hold a credential. Send outbound calls through `baukit-egress`, not a raw `reqwest::Client`. See [outbound calls and limits](outbound-and-limits.md).

## Step 4: run imports as durable jobs

`baukit-integrations` does not depend on `baukit-jobs`. The job runner owns leases, attempt caps and backoff. Your job handler loads the connection, calls `fetch_page`, and maps the error. This doctest is also in the crate README:

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

let limited = ConnectorError::new("rate_limited", RetryClass::RetryAfter(
    std::time::Duration::from_secs(30),
));
assert!(to_job_error(&limited).is_retryable());
assert!(!to_job_error(&ConnectorError::revoked("token_revoked")).is_retryable());

let page = ConnectorPage::with_cursor(vec!["record"], serde_json::json!({"after": "abc"}));
assert!(page.has_more());
```

Enqueue the next window only when `has_more()` is true. Keep `max_attempts` on every enqueued job. `retryable_after` overrides the delay but not the cap, so a provider that keeps asking for 30 seconds still ends in `failed` with `JobFailureReason::AttemptsExhausted`. The [jobs guide](jobs.md) covers the worker.

## Step 5: accept verified webhooks

`verify_webhook(headers, body)` checks the signature before it parses the body. A bad signature returns `WebhookVerificationError::InvalidSignature`. A good signature over an unusable payload returns `InvalidEvent`. Count the two under separate metric labels, because they mean different things during an incident.

A good delivery returns `VerifiedWebhook` with `provider`, `delivery_id`, `signature_digest`, `connection_id`, `job_kind` and `job_payload`. Your store decides whether `delivery_id` was seen before, with `signature_digest` as the fallback key. It reports `WebhookIngestResult::Accepted` or `Duplicate`.

The handler order is fixed by the [integration reliability contract](../platform/integration-reliability.md):

1. Read the bounded raw body.
2. Verify signature, timestamp window and provider context.
3. Deduplicate on the delivery id.
4. In one PostgreSQL transaction, insert the inbox row and call `PostgresJobStore::enqueue_in_transaction`.
5. Respond 2xx only after the commit.

Scope the inbox unique key by owner, connection and source event id, not the event id alone. The webhook route takes no bearer, so mount it inside the rate limiter. Add it to your threat model, as `lint/agent-rules.md` requires for every new trust boundary.

## Step 6: show connection health

After each failure, `ConnectionHealth::after_failure(class)` says what the owner should see. Doctest from the crate README:

```rust
use baukit_integrations::{ConnectionHealth, RetryClass};

assert_eq!(
    ConnectionHealth::after_failure(RetryClass::Unavailable),
    ConnectionHealth::Degraded,
);
assert!(ConnectionHealth::after_failure(RetryClass::Revoked).needs_owner_action());
```

Keep a `ConnectionStatus` per connection. Call `succeeded` to clear it and `failed` to record a code and the next retry time. It serializes as snake case, so it can go straight into your status response.

On the client, `connectionStateFromServer` from `@baukit/integrations-client` turns that response into a state, and `reduceConnectionState` applies user actions. It drops `providerDiagnostic` and accepts only a bounded snake_case code. Map `status` and `availableActions` to your own localized copy.

To check a stored token without running an import, implement `CredentialProbe`. It returns `CredentialProbeSuccess` with an `ExternalAccountId`, or a `CredentialProbeError` such as `Revoked`, `MissingScope` or `RateLimited`. Stop reading the response at `MAX_CREDENTIAL_PROBE_RESPONSE_BYTES`.

## Step 7: test with fake providers

`baukit_test::FakeConnector::new(provider, scenario)` scripts six scenarios: healthy, rate limited, unavailable, timeout, revoked and exhausted. The four transient ones fail once and then succeed, so a test proves the handler retries. `Exhausted` fails every time, which is how a test reaches the attempt cap.

Two conformance helpers run against your real adapters:

- `check_credential_probe_conformance` starts a scripted loopback server and runs your probe through the healthy, revoked, missing-scope, rate-limit, timeout, unavailable, malformed and oversized cases.
- `check_postgres_inbox_conformance` runs your webhook inbox against your PostgreSQL schema. It covers first delivery, replay, concurrent replay, rollbacks, and owner and source isolation.

For manual testing, the generated backend ships `docs/fake-providers.md`. It describes a WireMock container in a Compose override and a `FAKE_SCENARIO` variable with the same six scenario names.

Add one more test yourself: a callback where a second browser's cookie is denied. That is the integration-level version of the OAuth state test in step 2.

## Reference

- [baukit-integrations README](../../rust/crates/baukit-integrations/README.md)
- [baukit-credential-vault README](../../rust/crates/baukit-credential-vault/README.md)
- [baukit-jobs README](../../rust/crates/baukit-jobs/README.md)
- [baukit-test README](../../rust/crates/baukit-test/README.md)
- [@baukit/integrations-client README](../../typescript/packages/integrations-client/README.md)
- [Integration reliability contract](../platform/integration-reliability.md)
- [ADR 0002: integration connector contract](../adr/0002-integration-connector-contract.md)
- [ADR 0004: integration client contract](../adr/0004-integration-client-contract.md)
- [ADR 0018: credential vault records](../adr/0018-credential-vault-records.md)
- [ADR 0016: outbound HTTP through egress](../adr/0016-outbound-http-through-egress.md)
- [Template doc: fake providers](../../templates/backend/docs/fake-providers.md)
- OAuth state source: [`rust/crates/baukit-integrations/src/oauth_state/`](../../rust/crates/baukit-integrations/src/oauth_state/mod.rs)
