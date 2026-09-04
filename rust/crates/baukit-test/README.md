# baukit-test

`baukit-test` is the integration-test toolbox shared by Baukit services: Docker-backed PostgreSQL,
Redis, and Redis Sentinel fixtures, a mock OIDC issuer with JWT fixtures, and conformance assertions
that check a service actually follows the platform's contracts.

Add it under `[dev-dependencies]`. Nothing here belongs in a shipped binary.

## Conformance assertions

The most useful thing in the crate is the set of checks that a service still honors a contract it
opted into:

- `assert_ops_router_conformance`: the ops router serves the endpoints in the shape `baukit-ops`
  promises.
- `assert_metrics_conformance`: required metrics exist with the right names, types, and buckets.
- `assert_auth_router_conformance`: protected routes reject missing, malformed, and expired tokens.
- `assert_openapi_no_drift`: the committed schema matches the code.
- `check_product_profile_erasure_conformance`: a user-deletion path actually removes what it claims.
- `check_limit_boundaries`: a validator accepts `limit - 1` and `limit`, then rejects `limit + 1`.
- `check_update_at_capacity` and `check_soft_delete_capacity_reuse`: live-row caps allow updates and
  release capacity after soft deletion.
- `check_postgres_live_row_cap_conformance`: two creates race for the last slot, then the check
  verifies the live count, update behavior, soft-delete release, and stable limit code.
- `check_ingress_reason_code_parity`: every named write path returns the same stable reason code.
- `check_credential_probe_conformance`: a product provider adapter maps raw HTTP responses to the
  shared credential outcomes, preserves `Retry-After`, bounds response reads, and times out.
- `check_postgres_inbox_conformance`: a product inbox adapter preserves one domain effect and one
  outbox message across first delivery, replay, concurrent replay, and transaction failures.

A contract stated only in a document decays. Someone renames a metric, someone adds a route without
auth, someone changes an error envelope, and nothing fails until an alert stops firing months later.
These turn the document into a test. Each has a non-panicking `check_*` form returning a typed error
when a product wants a different report.

`audit_user_root_foreign_keys` walks the schema for foreign keys to the user root and reports mismatched
delete actions, which catches the table someone added without `ON DELETE CASCADE` before a deletion
request silently leaves rows behind.

## Container fixtures

```rust,no_run
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let postgres = baukit_test::start_postgres_with_migrations("migrations").await?;
let pool = sqlx::PgPool::connect(postgres.connection_url()).await?;
# let _ = pool;
# Ok(())
# }
```

`start_postgres` runs PostgreSQL 18 Alpine. It, `start_redis`, and `start_redis_sentinel` bind random
host ports and hand back a container that lives until the value drops, so tests run in parallel
without fighting over ports. The Sentinel fixture builds a real master/replica/sentinel topology on
its own network, which is the only way to test failover behavior honestly.

These need a running Docker daemon. Mark tests that use them `#[ignore]` and run them explicitly:

```bash
cargo test --manifest-path rust/Cargo.toml -- --include-ignored
```

## Auth fixtures

`MockOidcServer` serves discovery and JWKS documents and signs tokens the real `OidcVerifier` accepts,
so the whole verification path runs in a test without a live identity provider. `hs256_token`,
`rs256_token`, `rs256_token_with_key_id`, and `unsigned_token` build tokens from `JwtClaims`, including
the malformed ones you need for negative cases. `InMemoryApiTokenStore` implements `ApiTokenStore` for
tests that exercise personal access tokens. Call `fail_with` with `ApiTokenStoreError::Internal` or
`ApiTokenStoreError::PolicyRejected` to test both failure paths without a database adapter.

`FakeConnector` plays back scripted outbound-integration scenarios, including signature headers, for
testing retry and failure handling without a real upstream.

`ScriptedCredentialProbeHttp` is the lower-level fake for provider credential checks. It returns
queued status, header, body, or pending responses and records only a call count. It never retains
request headers, paths, bodies, or credentials. Pass product-authored responses to
`CredentialProbeConformanceCases`, then use `check_credential_probe_conformance` with a closure that
builds the product adapter against the supplied loopback origin. Baukit does not need a provider name
or a branch for provider-specific scope and response rules.

## Inbox and webhook fixtures

Implement `PostgresInboxPort` in a product integration test. The adapter maps `InboxScope` to the
product's owner, source, and event ID columns, then maps its stored result to `InboxReceipt`. Run
`check_postgres_inbox_conformance` against a fresh PostgreSQL database. The check races two exact
deliveries and verifies rollback after the inbox insert, domain write failure, and outbox write
failure. It also checks owner and source isolation and reads the outcome again after process-local
state is discarded. Inbox values that carry product identifiers, payloads, or outcomes do not
implement `Debug`.

`sign_webhook_hmac_sha256` and `verify_webhook_hmac_sha256` use the canonical signing bytes documented
in the integration reliability recipe. `ScriptedWebhookReceiver` records bounded requests without a
`Debug` implementation and returns queued statuses in order. Use it to test successful delivery,
`Retry-After`, permanent receiver responses, timeouts, stable request bodies, and idempotency headers.

These APIs are additive. Existing connector and credential-probe tests need no migration. Products
adopting the inbox check must use a uniqueness constraint over owner, source, and event ID. Products
adopting the signing helper must version their signature header and retain the previous verification
key for their documented rotation overlap.

## Resource limits

Products own their limits and reason codes. `baukit-test` checks the behavior without knowing either.
Use `check_limit_boundaries` with a payload builder and the product validator:

```rust
# tokio_test_block(async {
use baukit_core::limits::trimmed_unicode_scalar_count;
use baukit_test::check_limit_boundaries;

check_limit_boundaries(
    120,
    |length| "é".repeat(length),
    |text| async move {
        if trimmed_unicode_scalar_count(&text) <= 120 {
            Ok(())
        } else {
            Err("text_too_long")
        }
    },
)
.await?;
# Ok::<(), baukit_test::LimitsConformanceError>(())
# });
# fn tokio_test_block<F: std::future::Future>(future: F) -> F::Output {
#     tokio::runtime::Builder::new_current_thread()
#         .build()
#         .expect("runtime")
#         .block_on(future)
# }
```

Production code should import measurements and checks from `baukit_core::limits`. The
`trimmed_text_length` and `compact_document_bytes` names remain as compatibility aliases in
`baukit-test`, but both now call the `baukit-core` implementation.

Implement `LiveRowLimitAdapter` around a fresh owner or parent fixture. Run
`check_update_at_capacity` and `check_soft_delete_capacity_reuse` separately because each helper fills
the fixture to its cap. Use `NamedIngress` with `check_ingress_reason_code_parity` to invoke REST,
sync, import, and local write paths against the same invalid input. The caller-supplied extractor reads
the product's reason code from each output.

For a PostgreSQL cap, implement `PostgresLiveRowCapAdapter` around a clean scope and run
`check_postgres_live_row_cap_conformance`. The adapter uses `&self` so its two raced creates can take
separate connections from a pool. The check fills all but one slot, requires exactly one raced create
to succeed, updates at capacity, soft-deletes a row, and creates a replacement. It compares the
rejected create with the product's stable limit code without formatting the rest of the product error.
See the [PostgreSQL live-row cap recipe](../../../docs/platform/live-row-caps.md) for row-lock,
serializable, counter, and slot-constraint SQL.

The PostgreSQL API is additive. Existing `LiveRowLimitAdapter` implementations and sequential checks
remain available. Migrate a database-backed suite by adding a separate clean-scope adapter for the
race check; no existing helper call needs to change.

## Telemetry in tests

`init_test_tracing` installs a lightweight subscriber for tests that just need log output. It is not an
isolated telemetry runtime.

Real `baukit-telemetry` initialization is process-global and cannot be reset, even after shutdown. Every
assertion needing a real recorder, subscriber, or exporter has to live in one test per binary that
initializes telemetry exactly once. Split them across two tests and the second fails depending on
execution order, which is a miserable afternoon to debug from the symptom.

## Scope

This crate provides fixtures and contract checks. Products still decide their limits, persistence,
error types, and policy. The helpers only check the behavior a product declares.
