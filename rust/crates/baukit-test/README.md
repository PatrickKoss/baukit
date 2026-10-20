# baukit-test

`baukit-test` is the integration-test toolbox shared by Baukit services: Docker-backed PostgreSQL,
Redis, and Redis Sentinel fixtures, a mock OIDC issuer with JWT fixtures, and conformance assertions
that check a service actually follows the platform's contracts.

Add it under `[dev-dependencies]`. Nothing here belongs in a shipped binary.

## Testcontainers compatibility

Baukit uses testcontainers 0.28. testcontainers-modules 0.15 requires 0.27,
and the two versions require different exact bollard-stubs 1.x versions.
Products must replace testcontainers-modules with `GenericImage` to use this
version of baukit-test. The advanced fixture guards also use `GenericImage`.

```rust,no_run
use testcontainers::{GenericImage, ImageExt, core::{IntoContainerPort, WaitFor}, runners::AsyncRunner};

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let container = GenericImage::new("redis", "8.10.2-alpine")
    .with_exposed_port(6379_u16.tcp())
    .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
    .with_labels(baukit_test::container_labels())
    .start()
    .await?;
let port = container.get_host_port_ipv4(6379).await?;
let url = format!("redis://{}:{port}/", container.get_host().await?);
# drop((url, container));
# Ok(())
# }
```

## Conformance assertions

The most useful thing in the crate is the set of checks that a service still honors a contract it
opted into:

- `assert_ops_router_conformance`: the ops router serves the endpoints in the shape `baukit-ops`
  promises.
- `assert_metrics_conformance`: required metrics exist with the right names, types, and buckets.
- `assert_auth_router_conformance`: protected routes reject missing, malformed, and expired tokens.
- `assert_openapi_no_drift`: the committed schema matches the code.
- `assert_openapi_camel_case`: every property and path or query parameter name is camelCase.
- `assert_response_matches_openapi`: a real response has a documented status, media type, and a
  body that validates against the documented schema.
- `assert_request_matches_openapi`: a request body a test sends has a documented media type and
  validates against the operation's `requestBody` schema.
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
- `check_purge_horizon_conformance`: a product tombstone purge keeps a per-owner horizon that never
  drops, stays within its batch limit, and rejects stale pull cursors without losing a deletion to
  a concurrent pull.
- `check_replay_safe_mutation_conformance`: a product's keyed mutation applies its effect once,
  replays the stored result after a lost response or a restart, rejects a reused key with other
  input, leaves nothing after a rollback, and expires, cleans up, and erases its replay records.

A contract stated only in a document decays. Someone renames a metric, someone adds a route without
auth, someone changes an error envelope, and nothing fails until an alert stops firing months later.
These turn the document into a test. Each has a non-panicking `check_*` form returning a typed error
when a product wants a different report.

`audit_user_root_foreign_keys` walks the schema for foreign keys to the user root and reports mismatched
delete actions, which catches the table someone added without `ON DELETE CASCADE` before a deletion
request silently leaves rows behind.

## OpenAPI requests and responses

`assert_response_matches_openapi` checks one response a test received against the operation it
answers in the serialized document. Pass the documented path template, not the request URL:

```rust
use baukit_test::{ObservedResponse, assert_response_matches_openapi};
use serde_json::json;

let document = json!({
    "openapi": "3.1.0",
    "paths": {"/items/{id}": {"get": {"responses": {"200": {
        "description": "Item",
        "content": {"application/json": {"schema": {"$ref": "#/components/schemas/Item"}}}
    }}}}},
    "components": {"schemas": {"Item": {
        "type": "object",
        "required": ["id"],
        "properties": {"id": {"type": "string", "format": "uuid"}}
    }}}
});
let body = br#"{"id":"0190a6d8-8f43-7c1e-9b35-3f9d7c1b2a10"}"#;

assert_response_matches_openapi(
    &document,
    &ObservedResponse {
        method: "GET",
        path: "/items/{id}",
        status: 200,
        content_type: Some("application/json; charset=utf-8"),
        body,
    },
);
```

The status matches an exact code, then its class such as `4XX`, then `default`. Response `$ref`s
resolve inside the document. A body must be present exactly when the response documents content,
and it needs a `Content-Type`. The media type matches exactly, then `type/*`, then `*/*`. Bodies of
`application/json` and `+json` media types validate as JSON Schema 2020-12 against the document's
`components`, with `format` checked, so a malformed UUID or date-time fails. Other media types,
such as a PDF, only have to be documented. `check_response_matches_openapi` returns an
`OpenApiContractError` instead of panicking; a schema violation lists every failing value with its
JSON pointer.

`assert_request_matches_openapi` checks the body of a request a test sends against the
operation's `requestBody`, before or after the service answers it:

```rust
use baukit_test::{ObservedRequest, assert_request_matches_openapi};
use serde_json::json;

let document = json!({
    "openapi": "3.1.0",
    "paths": {"/items": {"post": {
        "requestBody": {"required": true, "content": {"application/json": {"schema": {
            "type": "object",
            "required": ["name"],
            "properties": {"name": {"type": "string"}}
        }}}},
        "responses": {"201": {"description": "Created"}}
    }}}
});

assert_request_matches_openapi(
    &document,
    &ObservedRequest {
        method: "POST",
        path: "/items",
        content_type: Some("application/json"),
        body: br#"{"name":"Gear"}"#,
    },
);
```

A `requestBody` `$ref` resolves through `components.requestBodies`. An empty body passes unless
the request body is `required`, and an operation without a `requestBody` rejects any body. Media
types and JSON bodies follow the response rules above, and `check_request_matches_openapi` returns
the same `OpenApiContractError`. Parameters and headers are not checked.

Build the document with `serde_json::to_value(openapi_document())` or read the committed
`openapi.json`, so the test checks the same document clients see.

## Container fixtures

```rust,no_run
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let postgres = baukit_test::start_postgres_with_migrations("migrations").await?;
let pool = sqlx::PgPool::connect(postgres.connection_url()).await?;
# let _ = pool;
# Ok(())
# }
```

`start_postgres` runs PostgreSQL 18.6 Alpine. It, `start_redis`, and `start_redis_sentinel` bind random
host ports and hand back a container that lives until the value drops, so tests run in parallel
without fighting over ports. The Sentinel fixture builds a real master/replica/sentinel topology on
its own network, which is the only way to test failover behavior honestly.

### Leaked containers

Baukit enables the testcontainers watchdog to stop and remove registered containers on
SIGINT, SIGTERM, and SIGQUIT. Normal fixture drops also remove containers. SIGKILL and OOM
kills cannot be caught. The watchdog can fail if Docker is unavailable or a registered
container was removed outside testcontainers.

Every Baukit container fixture has these labels:

- `dev.baukit.test=true` marks a test container.
- `dev.baukit.test.pid` identifies the owning process.
- `dev.baukit.test.pid-ns` contains the target of `/proc/self/ns/pid`, when readable.
- `dev.baukit.test.host` contains `/proc/sys/kernel/hostname`, or `/etc/hostname` as a
  fallback, when readable.
- `dev.baukit.test.binary` contains the executable's file name, when available.
- `dev.baukit.test.manifest-dir` contains `CARGO_MANIFEST_DIR` from the test process's
  environment, when set. It identifies the crate and worktree.
- `dev.baukit.test.started-at` records Unix seconds.

For a container started directly with testcontainers, apply `baukit_test::container_labels()`
with `ImageExt::with_labels` before `.start()`.

Run [`scripts/reap-test-containers.sh`](../../../scripts/reap-test-containers.sh) on the
local Linux Docker host to remove running or stopped containers whose owning process is
gone, or whose PID now belongs to a different binary. It touches only containers labelled
`dev.baukit.test=true` whose host and PID namespace labels both match the reaper's own.
It skips containers with missing or mismatched host or namespace labels, including tests
in other devcontainers or on another host sharing the daemon. It also skips cleanup and
reports why on stderr when `DOCKER_HOST` is set to anything other than a Unix socket, or
its own host name or PID namespace is unreadable. It preserves containers when the PID
is missing or invalid, or a live process's executable cannot be checked. Preview the
candidates first:

```sh
./scripts/reap-test-containers.sh --dry-run
```

For hourly cleanup, put these units in `/etc/systemd/system/`. Replace the script path
with your checkout's absolute path. The service needs Docker access and permission to
read the owning processes' `/proc/<pid>/exe` links. It must use the same host PID namespace
as the tests, have the same host name, and use a local Docker daemon.

`baukit-reap-test-containers.service`:

```ini
[Unit]
Description=Remove orphaned Baukit test containers

[Service]
Type=oneshot
ExecStart=/absolute/path/to/baukit/scripts/reap-test-containers.sh
```

`baukit-reap-test-containers.timer`:

```ini
[Unit]
Description=Check for orphaned Baukit test containers hourly

[Timer]
OnCalendar=hourly
Persistent=true

[Install]
WantedBy=timers.target
```

Enable the timer with `sudo systemctl enable --now baukit-reap-test-containers.timer`.

### PostgreSQL options

`PostgresTestOptions` builds the same container with three extra choices. `with_image` swaps the
image, for example `postgres` at `18.6-alpine3.23`. `with_app_role` creates a login role
without superuser or `BYPASSRLS` before migrations run, so row-level security policies apply to the
connection the product's code uses. `with_migrations` applies SQLx migrations as the `postgres`
superuser.

```rust,no_run
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
use baukit_test::{PostgresAppRole, PostgresTestOptions};

let postgres = PostgresTestOptions::new()
    .with_image("postgres", "18.6-alpine3.23")
    .with_app_role(PostgresAppRole::new("app_user", "app-secret")?)
    .with_migrations("migrations")
    .start()
    .await?;
let database = postgres.create_database().await?;
let admin = sqlx::PgPool::connect(database.connection_url()).await?;
let app = sqlx::PgPool::connect(database.app_connection_url().expect("app role is set")).await?;
# let _ = (admin, app);
database.drop_database().await?;
# Ok(())
# }
```

The app role gets `USAGE` on schema `public` and default `SELECT`, `INSERT`, `UPDATE`, and `DELETE`
on tables and `USAGE, SELECT` on sequences that the migrations create. A migration can still revoke
or narrow those grants. The role name must match `[a-z_][a-z0-9_]*`, and the password may use only
URL-unreserved characters because it ends up in the connection URL. When the role already exists,
Baukit leaves its password alone and fails with `PostgresTestError::InvalidAppRole` if the role is a
superuser or bypasses row-level security.

`create_database` gives one test its own database on a shared container, named
`baukit_test_<uuid>`, with the same grants and migrations. `drop_database` drops it with
`WITH (FORCE)` and reports errors; dropping the value without calling it runs the same statement on
a helper thread and ignores failures.

`PostgresTestDatabases` does the same against a server the test does not start, such as a
`DATABASE_URL` in CI. Pass it a URL for a role that can create databases and roles. Every database
it creates is dropped when its `PostgresTestDatabase` drops, so a shared server does not collect
test databases.

```rust,no_run
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
use baukit_test::PostgresTestDatabases;

let admin_url = std::env::var("DATABASE_URL")?;
let database = PostgresTestDatabases::new(admin_url)
    .with_migrations("migrations")
    .create()
    .await?;
let pool = sqlx::PgPool::connect(database.connection_url()).await?;
# let _ = pool;
# Ok(())
# }
```

These need a running Docker daemon. Mark tests that use them `#[ignore]` and run them explicitly:

```bash
cargo test --manifest-path rust/Cargo.toml -- --include-ignored
```

## Auth fixtures

`MockOidcServer` serves discovery and JWKS documents and signs tokens the real `OidcVerifier` accepts,
so the whole verification path runs in a test without a live identity provider. It also tests the
verifier's JWKS cache. `jwks_url` feeds `OidcVerifier::from_jwks_uri` when a test skips discovery.
`jwks_request_count` counts JWKS requests, and `set_jwks_delay` holds each response back, so a test
can prove that concurrent refreshes share one request, that a cache TTL triggers a refetch, and that
a timeout fires. `mint_with_key_id` signs with the active key under a `kid` the JWKS does not
publish, for unknown-key refresh and negative caching:

```rust,no_run
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
use std::time::Duration;

use baukit_auth::{OidcConfig, OidcVerifier};
use baukit_test::MockOidcServer;

let server = MockOidcServer::start().await?;
let verifier = OidcVerifier::from_jwks_uri(OidcConfig::new(server.issuer(), "api")?, server.jwks_url())?;
let claims = server.claims("user-123", "api", Duration::from_secs(60))?;
let unknown = server.mint_with_key_id(&claims, "unpublished-key")?;
assert!(verifier.verify(&unknown).await.is_err());
assert!(verifier.verify(&unknown).await.is_err());
assert_eq!(server.jwks_request_count(), 1);
# Ok(())
# }
```

`hs256_token`,
`rs256_token`, `rs256_token_with_key_id`, and `unsigned_token` build tokens from `JwtClaims`, including
the malformed ones you need for negative cases. `InMemoryApiTokenStore` implements `ApiTokenStore` for
tests that exercise personal access tokens. It stores grants with the token and, like the PostgreSQL
store, never moves `last_used_at` backwards. Call `fail_with` with `ApiTokenStoreError::Internal` or
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

The `baukit-webhook-v1` signature lives in `baukit_core::webhook_signature` (feature
`webhook-signature`), so senders and receivers use it at runtime and tests check against the same
code. `ScriptedWebhookReceiver` records bounded requests without a
`Debug` implementation and returns queued statuses in order. Use it to test successful delivery,
`Retry-After`, permanent receiver responses, timeouts, stable request bodies, and idempotency headers.

These APIs are additive. Existing connector and credential-probe tests need no migration. Products
adopting the inbox check must use a uniqueness constraint over owner, source, and event ID. Products
adopting the signature must version their signature header and retain the previous verification
key for their documented rotation overlap.

## Purge-horizon fixtures

Implement `PurgeHorizonAdapter` in a product integration test against a fresh PostgreSQL database,
then run `check_purge_horizon_conformance`. The adapter creates owners, writes live rows and
tombstones with a given `deleted_at`, runs one purge batch, pulls from a cursor, reads the horizon,
and erases an owner. Its `purge_batch` returns the number of rows it removed, and `pull` returns
`PurgeHorizonPull::ResyncRequired` for the product's 409 response.

The check drains everything already expired before each case, so one database can serve every
case. It covers horizon monotonicity, cursor zero and cursors at, above, and below the horizon,
batch limits, owner isolation, and erasure. For the race, `pull` must call `PullPause::reached`
after its cursor check and before it reads rows, inside the same transaction. The check purges the
owner while the pull is paused. A pull that passed its cursor check must still return the purged
tombstone, or the purge must wait for the pull to finish. A product that reads the horizon in one
transaction and rows in another fails this case.

## Replay-safe mutation fixtures

Implement `ReplaySafeMutationAdapter` in a product integration test against a fresh PostgreSQL
database, with the product's real roles and row-level security, then run
`check_replay_safe_mutation_conformance` with four JSON bodies in `ReplayConformanceInputs`. The
protocol it checks is [replay-safe mutations](../../../docs/platform/replay-safe-mutations.md).

`execute` runs one keyed request through the product's replay path and returns
`ReplayOutcome::Applied`, `Replayed`, `Conflict`, or `InProgress`, each with the stored
`ReplaySnapshot` where there is one. Inside the transaction, after the effect and the replay record
are written and before `COMMIT`, it calls `CommitCheckpoint::reached`. When that returns
`InjectedRollback`, the adapter rolls back and returns an error. The other methods count effects
and replay records, move an owner's records past the horizon, run one cleanup batch with a limit,
erase an owner, and drop process-local caches as a restart would.

The cases are a lost response, an equivalent body with reordered members, changed input under one
key, two simultaneous requests with one key, owner and operation isolation, a rollback before
commit, a crash after commit, expiry, bounded cleanup, and erasure. For the race, the check pauses
the first request at the checkpoint and sends the second. The second must replay the first
result or report `InProgress`, and only one effect may commit. The cleanup case runs
`purge_expired` in the product's cleanup role; a job that runs under `FORCE ROW LEVEL SECURITY`
without a matching policy deletes nothing and fails. Violation messages name the case and never
contain adapter errors, keys, or bodies.

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

## Data subject service conformance

Enable `data-subject` and implement `DataSubjectConformanceAdapter` for an
isolated product fixture. Run the complete router, including authentication:

```rust,no_run
# #[cfg(feature = "data-subject")]
# async fn example<A: baukit_test::DataSubjectConformanceAdapter>(
#     router: &axum::Router, adapter: &mut A,
# ) -> Result<(), baukit_test::DataSubjectConformanceError> {
use baukit_test::{DataSubjectEndpoint, check_data_subject_conformance};

check_data_subject_conformance(DataSubjectEndpoint::Router(router), adapter).await?;
# Ok(())
# }
```

Use `DataSubjectEndpoint::BaseUrl(base_url)` for a listening service.
`assert_data_subject_conformance` takes the same arguments and panics on failure.
`DataSubjectConformanceError` names the failed requirement without exposing
lookup values, credentials, responses or adapter error messages.

The adapter uses `#[async_trait::async_trait]`. Its `mint_token` method accepts
`DataSubjectTestClient` and requested scopes. Supply a primary allowed OAuth
client, another allowed client, a disallowed client and an API token. Its
`seed_subject` method accepts the operation kind, `DataSubjectTestSubject` and
an opaque reference. Return independent identifiers for matched subjects,
unmatched identifiers and subjects with legal restrictions. Return `None` only
when restrictions are unsupported. Matched exports must produce at least one
part. Use distinctive lookup values absent from export content.

`run_pending_work` drains the product's workers until the requests finish.
Keep workers under fixture control until this call so pending export downloads
can be tested. The checker never sleeps or polls for completion. It checks the
advertised operations through HTTP, including auth and scopes, validation,
replay, conflict, ownership, signed receipts and export digests. It scans every
response and decoded receipt for submitted lookup values. Unlisted operation
kinds are skipped. See the [adapter requirements](../../../docs/platform/data-subject-contract.md#8-conformance)
for the full contract. Storage and log privacy need separate product tests.

## Data subject client tests

Enable the `data-subject` feature for `DataSubjectTargetServer`. It binds an
ephemeral loopback port and uses the real `ReceiptSigner`. Register bearer
credentials with a client ID and scopes, then configure outcomes by opaque
subject reference. Every new POST returns pending; ordinary outcomes finish on
the first status GET. `PendingFor` adds pending polls without advancing on replay.

```rust,no_run
# #[cfg(feature = "data-subject")]
# async fn example() -> Result<(), Box<dyn std::error::Error>> {
use baukit_test::{DataSubjectOutcome, DataSubjectScript, DataSubjectFault, DataSubjectTargetServer};
use std::time::Duration;

let target = DataSubjectTargetServer::start().await?;
let token = target.register_token("test-bearer", "privacy-tool", ["data-subject.erase"]);
target.set_script("opaque-subject-reference", DataSubjectScript::new(
    DataSubjectOutcome::NotFound,
).with_fault(DataSubjectFault::ServerError { status: 503, times: 1 }))?;

let discovery = target.discovery();
let verifier = baukit_data_subject::ReceiptVerifier::from_discovery(&discovery)?;
let client_base_url = target.base_url();
# let _ = (token, verifier, client_base_url);
target.advance_time(Duration::from_secs(7 * 24 * 60 * 60))?;
# Ok(())
# }
```

Pass `base_url()` to the client under test. `set_default_script` sets the fallback.
Outcomes include category completion, no match, legal retention, failure, pending
polls and JSON exports. Export parts are numbered starting at one and include
signed digests and byte lengths. `DataSubjectTargetOptions` configures discovery
bounds, service identity and retention. `advance_time` exercises expiry and purge
without sleeping.

`DataSubjectFault` also supports rate limits with `Retry-After`, response delays,
unadvertised signing keys, incorrect key IDs, payload tampering, a signed wrong
subject reference and export digest mismatches. `push_fault` adds a fault to an
explicit subject script after POST to test polling or download retries. Delays
apply after processing, so retrying a timed-out POST reconciles the accepted
operation. `rotate_key` publishes a new current key, retains previous keys and
signs later completions with the new key.

`received_requests()` returns method, path, idempotency key, parsed JSON body and
the `DataSubjectTokenId` returned by registration. It never captures the bearer
credential. Observations have no `Debug` or serialization implementation; keep
their private bodies in memory. Tokens for the same client ID share operations
and replay records. Dropping the server shuts down its listener, connections and
delayed responses.

## Mock OAuth grants

`MockOidcServer` supports `refresh_token`, `authorization_code`, and
`client_credentials` at its discovered token endpoint. It does not serve an
interactive authorize endpoint. Use `issue_authorization_code(subject,
client_id, redirect_uri, code_challenge)` as the test's login step. Pass the
unpadded base64url SHA-256 challenge of a PKCE verifier, then exchange the code
with `client_id`, `redirect_uri`, and `code_verifier`. Each code expires after
five minutes and can be exchanged once. Failed proof or binding checks also
consume it. The access token has the client ID as its audience and `azp`, and
the refresh token works with `refresh_session`.

Use `register_client_credentials(client_id, client_secret, audience)` to
register a machine client. Send HTTP Basic authentication or the form's
`client_id` and `client_secret` with `grant_type=client_credentials`. The
response has a signed five-minute access token with the supplied audience,
client ID as its subject, and `azp` and `client_id` claims. It has no refresh
token. Unknown clients and incorrect secrets return `invalid_client`.

## Suite peers

Enable the `suite` feature for `suite::fixture`, `ScriptedSuiteReceiver` and
`InProcessSuitePeer`. Mount the other app's raw suite router on the in-process
peer before sending an exchange, delivery or revoke. It runs the real router
without opening a listening socket. Use the scripted receiver to check signed
bytes, response classes and timeouts over HTTP.

The crate embeds the JSON vectors and `SHA256SUMS` so published test kits can
load them without the Baukit checkout. Its copy lives in `src/suite/fixtures/v1`.
When updating the canonical `fixtures/suite-events/v1` directory, update this
copy too. The suite checksum test compares every packaged file with the canonical
version.
