# Build on the generated backend

`baukit new --backend` gives you a Rust workspace split into ports and adapters, with an items
resource wired end to end. This page shows where each layer lives, how a request travels through
it, and the steps to add your own resource. At the end you can add an endpoint that stays owner
scoped, appears in the OpenAPI document and passes the drift test.

## Layout

The template is `templates/backend/backend/`. In a product named `notes`:

| Crate | Holds | Depends on |
| --- | --- | --- |
| `notes-domain` | Entities such as `Item`, and limits. No framework code. | nothing baukit |
| `notes-ports` | Traits such as `ItemRepository` and `UserRepository`. | domain |
| `notes-services` | Use cases such as `ItemService`, built on the port traits. | domain, ports |
| `notes-api` | axum handlers, DTOs, `routes`, `finalize_api` and `openapi_document`. | services, `baukit-http`, `baukit-openapi` |
| `notes-postgres` | sqlx adapters for the ports. | ports, sqlx |
| `notes-worker` | Job handlers, with `--worker`. | services, `baukit-jobs` |
| `notes-mcp` | MCP tools, with `--mcp`. | services, `baukit-mcp` |
| `notes-bin` | Binaries `api`, `migrate`, `openapi` and `worker`, config and composition. | everything |

Migrations live in `backend/migrations`, the integration tests in `backend/tests`, and the fuzz
targets in `backend/fuzz`. [ADR 0009](../adr/0009-hexagonal-generated-backend.md) records the
split.

## How a request is served

`notes-bin/src/bin/api.rs` loads config through `ConfigLoader`, starts telemetry with
`TelemetryBuilder`, connects the adapters, and calls `compose_api`. The composition puts
authentication and the rate limiter around `routes(state)` from the API crate, then
`finalize_api` adds the baukit HTTP layers last, so CORS, request IDs and
`Cache-Control: private, no-store` cover 401 and 429 answers too.

`serve_listener_pair` from `baukit-runtime` runs two listeners. The API listens on 8080. The ops
listener on 9090 serves `/healthz`, `/readyz`, `/metrics` and `/buildinfo` and has none of the
public layers. [ADR 0011](../adr/0011-separate-api-and-ops-listeners.md) explains why.

## Overload and shutdown

The HTTP layers bound every wait:

- A request that gets no concurrency permit within 1 s (`HttpOptions::with_queue_timeout`) is shed
  with `503 unavailable` and `Retry-After: 1`, and counts in
  `http_requests_shed_total{reason="concurrency"}`.
- Every request carries a deadline of its start plus `http.request_timeout`.
  `baukit_http::current_deadline()` returns it, and `GuardedClient` caps outbound timeouts with
  it. The `route_timeout(duration)` layer tightens it for one route and answers `504` on expiry.
- `ServerOptions` closes connections that send no headers within 10 s or sit idle for 60 s, and
  caps open connections at 4096.
- `ApiError::unavailable(retry_after)` answers `503` with `Retry-After` when a dependency is down
  inside a request.

`api.rs` builds the shutdown token with `ShutdownToken::with_budget(DrainBudget::new(drain_timeout,
request_timeout)?)`. The budget fails at startup unless the request timeout plus a 3 s flush
reserve fits inside `shutdown.drain_timeout`, which defaults to 35 s. The listener drain and
supervised tasks stop when only the reserve is left. The telemetry flush runs in `run_flush`, the
one call that may use the reserve.

The `baukit-app` chart derives `SHUTDOWN__DRAIN_TIMEOUT` from the pod grace period. The API
default is `api.terminationGracePeriodSeconds: 45` minus the 5 s preStop sleep minus 5 s, so
35 s. With the default request timeout and preStop sleep, the API needs a grace period of at
least 44 s. The [overload and drain migration guide](../migrations/http-overload-and-drain.md)
has the details.

`examples/minimal-api` is the same composition in one small crate, without auth or a database.
Its `src/main.rs` and `src/lib.rs` are the shortest working reference:

```rust
// examples/minimal-api/src/lib.rs
pub fn api_router(
    state: AppState,
    config: &HttpConfig,
    environment: Environment,
) -> Result<Router, HttpOptionsError> {
    let router = Router::new()
        .route("/notes", post(create_note).get(list_notes))
        .route("/notes/{id}", get(get_note))
        .route("/fail", get(fail_validation))
        .with_state(state);
    Ok(finalize(
        router,
        HttpOptions::from_config_in(config, environment)?,
    ))
}
```

## Configuration

Config comes from `config/*.toml` and environment variables with the product prefix, two
underscores between levels: `NOTES__DATABASE__URL`. `NOTES_ENVIRONMENT` picks the environment
and defaults to `local`. Without a database URL the API falls back to an in-memory repository.
Migrations run only through `make migrate`, never at startup.

At startup the API retries PostgreSQL, Redis and the identity provider for `startup.budget`
seconds, 60 by default, with jittered backoff, then fails. Keep it inside your startup probe
window. `startup` is a reserved top-level key, so a product field of that name stops receiving
values. Pools come from `baukit_ops::connect_with_backoff(&config.database, budget)`, which also
sends the session timeouts from `DatabaseConfig`:

| Key | Default (seconds) |
| --- | --- |
| `database.statement_timeout` | 29, must stay below `http.request_timeout` |
| `database.lock_timeout` | 5 |
| `database.idle_in_transaction_session_timeout` | 60 |
| `database.max_lifetime` | 1800 |
| `database.idle_timeout` | 600 |

The `migrate` binary uses `connect_for_migrations`, which sends only `lock_timeout`. On shared
platform Postgres the pooler drops these parameters, so set them on the owner role as the
[startup budget migration guide](../migrations/startup-budget-and-postgres-timeouts.md) shows.

## Add a resource

Follow the items resource. Each step names the template file to copy from.

1. Domain. Add the entity to `notes-domain/src/lib.rs`, from `__app__-domain/src/lib.rs`.
2. Port. Add a repository trait to `notes-ports/src/lib.rs`. Every method that reads or writes
   tenant data takes `owner_id`, like `ItemRepository::get(owner_id, id)`.
3. Migration. Add `backend/migrations/NNNN_<name>.sql`. Migrations are forward only.
4. Adapter. Implement the trait in `notes-postgres`. Put the owner in the `WHERE` clause:

   ```sql
   -- templates/backend/backend/crates/__app__-postgres/src/lib.rs
   SELECT id, owner_id, name FROM items WHERE owner_id = $1 AND id = $2
   ```

   Loading by id and comparing the owner in Rust is a finding.
5. Service. Add the use case to `notes-services`, taking `Arc<dyn YourRepository>`.
6. Handler. Add the handler and DTOs to `notes-api/src/lib.rs` and register the route inside
   `routes`. Take `principal: Principal` for the owner and return `ApiError` for failures.
   Annotate it with `utoipa::path` and add it to `openapi_document`.
7. Wire it. Build the adapter and service in `Backend::connect` in `notes-bin/src/bin/api.rs`, next to `ItemService::new`, and add it to `ApiState`.
8. Regenerate and test:

   ```sh
   make openapi          # rewrites backend/openapi.json
   make openapi-client   # rewrites the TypeScript declarations
   make test             # includes the Docker-backed tests
   ```

Every route that takes an id needs a test where a second principal is denied. The
`baukit-add-endpoint` agent skill has the full checklist.

## Errors and the wire contract

Handlers return `baukit_http::ApiError`: built with `validation`, `not_found`, `conflict`, `internal` and the
other constructors. The body is one envelope:

```json
{ "error": { "code": "validation_failed", "message": "...", "requestId": "...", "details": [] } }
```

Source: `examples/minimal-api/tests/service.rs`. JSON names are camelCase.
[ADR 0012](../adr/0012-http-wire-contract.md) is the full contract.

## OpenAPI drift

The `openapi` binary writes `backend/openapi.json`. `backend/tests/openapi_drift.rs` fails when
the committed file differs from `openapi_document()`, and when a name is not camelCase:

```rust
// templates/backend/backend/tests/openapi_drift.rs
#[test]
fn committed_openapi_has_no_drift() {
    baukit_test::assert_openapi_no_drift(
        &openapi_document(),
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../openapi.json"),
    );
}
```

Add names a standard fixes, such as `access_token`, to `STANDARD_DEFINED_NAMES` in that file.
Each path in `openapi.consumers` gets declarations from `baukit generate openapi-client`. The
product's `docs/openapi-drift.md` covers the CI job.

## Outbound calls

Outbound HTTP goes through `baukit-egress`. Do not build a raw `reqwest::Client`.
[ADR 0016](../adr/0016-outbound-http-through-egress.md) explains the allowlist and the private
address rules.

## Reference

- `templates/backend/backend/`
- [`examples/minimal-api`](../../examples/minimal-api/README.md)
- [`baukit.toml` reference](manifest.md)
- [Migration: overload shedding and the drain budget](../migrations/http-overload-and-drain.md)
- [Migration: startup budget and PostgreSQL timeouts](../migrations/startup-budget-and-postgres-timeouts.md)
- [ADR 0009: hexagonal generated backend](../adr/0009-hexagonal-generated-backend.md)
- [ADR 0011: separate API and ops listeners](../adr/0011-separate-api-and-ops-listeners.md)
- [ADR 0012: HTTP wire contract](../adr/0012-http-wire-contract.md)
- [ADR 0016: outbound HTTP through egress](../adr/0016-outbound-http-through-egress.md)
- [ADR 0029: observability contract](../adr/0029-observability-contract.md)
- [ADR 0031: OpenAPI mirrors](../adr/0031-openapi-mirrors.md)
