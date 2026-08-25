# ADR 0002: Integration connector contract

## Status

Accepted, 2026-08-22.

## Context

Fitness Tracker talks to seven external health providers: Strava, Fitbit, Oura,
Polar, WHOOP, Withings, and Google Health. Each adapter is between 800 and 1,800
lines and each one knows things nobody else can know: the OAuth dance, the scope
names, the cursor encoding, the response models, and which record in the payload
counts as a duplicate of which.

Underneath that provider knowledge the seven adapters do the same four things.
They fetch one cursor-paged window of external records for a claimed job. They
classify the failure that came back. They verify a signed webhook and reduce it
to a deduplicated delivery. They report the connection's health so the product
can decide between retrying and asking the owner to reconnect.

The duplication is literal, not thematic. `map_status` and `map_transport_error`
appear once per adapter and the bodies are byte-identical apart from the error
type's prefix:

```rust
fn map_status(status: StatusCode, headers: &HeaderMap) -> OuraProviderError {
    if status == StatusCode::TOO_MANY_REQUESTS {
        // read Retry-After, fall back to None
    } else if status.is_server_error() {
        OuraProviderError::Unavailable
    } else {
        OuraProviderError::InvalidData
    }
}
```

Swap `Oura` for `Strava` and the function is the same. Baukit already absorbed
that half: `baukit_http::classify_http_status` and `classify_transport_error`
return a `RetryClass`, and the seven copies collapse into calls.

What Baukit has not absorbed is the seam above those functions. Fitness Tracker
had already built it, in `ports/integration_job_store.rs`: an
`IntegrationConnector` trait with `provider_id`, `verify_webhook`, and
`fetch_page`, returning a `ConnectorPage` or a `ConnectorError` carrying its own
three-variant `RetryClass`. The scripted `FakeIntegrationConnector` next to it
runs six scenarios (healthy, rate limited, unavailable, timeout, revoked,
exhausted) and is what makes the job dispatch tests possible without a network.

That trait is small, it names no provider, and it is the piece a second product
would otherwise retype. Its neighbours in the same file are not: the
`IntegrationJobStore` trait, the normalized record and link writes, and the
recovery vocabulary are all bound to Fitness Tracker's tables.

The obvious home for the seam is `baukit-jobs`, since a connector is driven by a
job runner. That would be wrong. `baukit-jobs` pulls in SQLx and a PostgreSQL
store. A product that wants to describe a connector shape, or a client that only
needs to name the error classes, would inherit a database driver it never calls.

## Decision

- Add `baukit-integrations` as its own contract-only crate. It holds the
  `IntegrationConnector` trait, `ConnectorPage<T>`, `ConnectorError`,
  `ClaimedConnectorJob`, the webhook verification and dedupe shapes, and the
  connection health vocabulary. It depends on `baukit-http` for `RetryClass`,
  plus serde, thiserror, chrono, and uuid. No SQLx and no HTTP client.
- Re-export `baukit_http::RetryClass` rather than defining a second retry enum.
  One classification vocabulary spans the HTTP boundary, the connector port, and
  the runner's requeue decision. Fitness Tracker's three-variant local copy
  becomes the six-variant shared one, which also covers `Revoked` and
  `Permanent` without a separate `ConnectorError` arm doing the same work twice.
- Use boxed futures on the trait, matching `CredentialVault` and `PushSender`.
  No `async-trait` dependency, and the trait stays object safe so a registry can
  hold `Arc<dyn IntegrationConnector<Record = _>>`.
- Compose with `baukit-jobs` instead of replacing it. A job payload names the
  connection, the runner calls `fetch_page`, and `RetryClass::is_retryable` plus
  `retry_after` decide between `JobError::retryable_after`,
  `JobError::retryable`, and `JobError::permanent`. `baukit-integrations` does
  not depend on `baukit-jobs`; the mapping lives in the product's handler and in
  this crate's README example.
- Ship `FakeConnector` in `baukit-test`, next to `MockOidcServer` and
  `InMemoryApiTokenStore`, scripting the six scenarios the product fake runs.
  `baukit-test` gains a dependency on `baukit-integrations`; the reverse
  dependency does not exist, so there is no cycle and no `test-support` feature.

## What stays product-side

- Per-provider persistence. The connection table, the job table, the OAuth
  session rows, the normalized record and link writes, and the transactional
  completion path are product schema decisions.
- Token exchange and refresh. Authorization URLs, PKCE, scope sets, and
  subscription registration differ per provider and per product registration.
- The provider-specific `TokenSet`, fetch request, fetch page, and provider error
  enums. `OuraFetchKind`, `WhoopFetchRequest`, and their siblings stay where the
  provider knowledge is.
- Dedupe heuristics. Deciding that a Withings weight sample and a Google Health
  weight sample are the same measurement is a product judgement about product
  data. The contract carries the delivery identity a connector already computed;
  it does not decide what counts as a duplicate record.
- The record type itself. `ConnectorPage<T>` is generic precisely so the product
  keeps its normalized shape.

## Consequences

A second product implementing a connector writes the provider knowledge and
nothing else. It gets the trait, the page shape, the error vocabulary, and a
scripted fake that already covers rate limiting, revocation, and attempt
exhaustion, which is the part teams usually skip testing.

The crate is contract-only, so it cannot be wrong in an expensive way. There is
no adapter to keep current with a vendor API and no migration to run. The cost
of getting the shape slightly wrong is a signature change in one small crate.

Fitness Tracker's `IntegrationConnector` and the shared one will coexist until
the product migrates. That is a rename plus deleting a local `RetryClass`, not a
rewrite, because the shared trait was modelled on the product one.

The three-variant to six-variant `RetryClass` change means a product matching
exhaustively on the class must handle `Revoked` and `Permanent`. Both were
already expressed as separate `ConnectorError` variants, so the match arms exist;
they move.

`baukit-integrations` does not solve OAuth. Seven adapters share a token refresh
shape too, and this ADR deliberately leaves it alone. Refresh interacts with the
credential vault, with per-provider expiry semantics, and with the product's
reconnect UX. One product is not enough evidence.

## Revision to the integration reliability contract

`docs/platform/integration-reliability.md` said:

> A collection of providers in one domain is not evidence for a shared connector
> framework.

It now says:

> A collection of providers in one domain is evidence for a shared connector
> *contract* but not for a shared connector *framework*. `baukit-integrations`
> owns the port shape, the retry vocabulary, and the scripted fake; provider
> resources, OAuth models and recovery, webhook schemas, cursor formats,
> credentials, normalized entities, and localized product UX remain
> product-local.
