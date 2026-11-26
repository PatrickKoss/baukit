# ADR 0002: Integration connectors share one contract crate

## Status
Amended 2026-11-27.

## Context

A consumer product talks to many external health providers. Each adapter holds knowledge nobody
else has: the OAuth flow, scope names, cursor encoding, response models and duplicate rules. Under
that knowledge every adapter does the same four things. It fetches one cursor-paged window for a
claimed job, classifies the failure, verifies a signed webhook and reduces it to a deduplicated
delivery, and reports connection health so the product can choose between retry and reconnect.

The status and transport error mapping was byte-identical across adapters, and baukit absorbed it as
`baukit_http::classify_http_status` and `classify_transport_error`. The seam above it, a connector
trait with a scripted fake, was the piece a second product would retype. It could not live in
`baukit-jobs`, because that crate pulls in SQLx and a PostgreSQL store that a contract user never calls.

## Decision

- `baukit-integrations` is its own crate. With default features it is contract only: the
  `IntegrationConnector` trait, `ConnectorPage<T>`, `ConnectorError`, `ClaimedConnectorJob`, the
  webhook shapes (`VerifiedWebhook`, `WebhookVerificationError`, `WebhookIngestResult`) and the
  health vocabulary (`ConnectionHealth`, `ConnectionStatus`). Its default dependencies are
  `baukit-http`, serde, serde_json, thiserror, jiff and uuid. There is no SQLx and no HTTP client.
- It re-exports `baukit_http::RetryClass`, so one retry vocabulary spans HTTP, the connector port
  and the job runner's requeue decision.
- The trait returns boxed futures and stays object safe, so a registry can hold
  `Arc<dyn IntegrationConnector<Record = _>>`. It does not use `async-trait`.
- The `CredentialProbe` port checks one provider credential before a connection is stored or
  resumed. It returns an `ExternalAccountId`, which has no `Display` and a redacted `Debug`.
  Adapters stop reading at `MAX_CREDENTIAL_PROBE_RESPONSE_BYTES` (64 KiB) and map failures to
  `CredentialProbeError`.
- Optional features add the server side of provider OAuth state. `oauth-state` adds
  `OAuthStateService`, `OAuthStateStore` and `InMemoryOAuthStateStore`. They hold single-use state
  bound to a browser cookie, with optional PKCE and a `DEFAULT_OAUTH_STATE_TTL` of 600 seconds.
  `postgres` adds `PostgresOAuthStateStore`, and `axum` adds cookie helpers for routes.
- The crate composes with `baukit-jobs` and does not depend on it. The mapping from `RetryClass` to
  `JobError` lives in the product's handler.
- `baukit-test` ships `FakeConnector` with scripted `FakeConnectorScenario` values and
  `ScriptedCredentialProbeHttp` for probe adapters.
- Products keep per-provider persistence, token exchange and refresh, provider-specific request
  and error types, dedupe heuristics and the normalized record type.

The connector registry uses `dyn` dispatch. The static dispatch review of Rust ports has not run
yet, so this choice may change.

## Enforced by

- `rust/crates/baukit-integrations/src/oauth_state/tests.rs`:
  `another_browser_is_rejected_and_burns_the_state`, `second_complete_is_already_used`,
  `expired_state_is_expired_and_burned`.
- `rust/crates/baukit-integrations/tests/oauth_state_postgres.rs`:
  `concurrent_completes_have_exactly_one_winner`, a Docker-gated test.
- `rust/crates/baukit-integrations/src/credential_probe.rs`:
  `external_account_ids_are_bounded_and_redacted`.
- Nothing checks that the default feature set stays free of SQLx and HTTP clients. A
  `cargo tree --no-default-features` check like `scripts/check-suite-features.sh` would.

## Consequences

A second product writes the provider knowledge and nothing else. It gets the trait, the page shape,
the error vocabulary, OAuth state storage and a fake that covers rate limiting, revocation and
attempt exhaustion. Baukit still does not solve token refresh, because refresh depends on the
credential vault, per-provider expiry and the product's reconnect flow.

## History

- 2026-08-22: accepted. The file entered this repository with baukit 0.1.0, commit `af6e2b4f`.
  The connector contract replaced the integration reliability contract's line that a provider
  collection is no evidence for shared code.
- 2026-09-04, commit `5b01e658`: added the credential probe contract.
- 2026-11-07, commit `187d405c`: added the `oauth-state` feature with PKCE.
- 2026-11-17, commit `3385d407`: chrono replaced by jiff across baukit crates.
- 2026-11-27: amended to the current features and the probe port.
