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
tests that exercise personal access tokens.

`FakeConnector` plays back scripted outbound-integration scenarios, including signature headers, for
testing retry and failure handling without a real upstream.

## Telemetry in tests

`init_test_tracing` installs a lightweight subscriber for tests that just need log output. It is not an
isolated telemetry runtime.

Real `baukit-telemetry` initialization is process-global and cannot be reset, even after shutdown. Every
assertion needing a real recorder, subscriber, or exporter has to live in one test per binary that
initializes telemetry exactly once. Split them across two tests and the second fails depending on
execution order, which is a miserable afternoon to debug from the symptom.

## Scope

Fixtures and contract checks, no product test helpers. Deciding what your service should do is your
test's job; this crate checks that it still does what the platform requires.
