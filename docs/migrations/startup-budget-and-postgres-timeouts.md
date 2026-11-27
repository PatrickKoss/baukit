# Startup budget, PostgreSQL session timeouts and the shared pooler

0.32.0 lets a process wait for PostgreSQL, Redis and the identity provider at startup instead of
crashing on the first refused connection. It also puts session timeouts on every pooled PostgreSQL
connection, and adds a PgBouncer pooler to the shared platform Postgres.

## The `startup` section

`baukit-config` reads a new top-level `startup` section:

```toml
[startup]
budget = 60
```

`budget` is in integer seconds, defaults to 60 and must be non-zero. It is how long each dependency
connect retries, with full jitter from 250 ms to 5 s, before startup fails. Keep it inside your startup
probe window. `startup` is now a reserved top-level key: the loader strips it before it deserializes the product
config, so a product field named `startup` at the top level stops receiving values. Rename it.

## `DatabaseConfig`

`DatabaseConfig` gains five fields, all integer seconds:

| Field | Default |
| --- | --- |
| `statement_timeout` | 29 |
| `lock_timeout` | 5 |
| `idle_in_transaction_session_timeout` | 60 |
| `max_lifetime` | 1800 |
| `idle_timeout` | 600 |

Code that builds `DatabaseConfig { .. }` by hand must add the fields or end with
`..DatabaseConfig::default()`. Validation rejects zero for each of them, and rejects a
`statement_timeout` that is not below `http.request_timeout`. The type moved to its own module and is
still re-exported at the crate root.

## Connecting

Build pools with `baukit-ops` instead of `PgPoolOptions`, so the pool uses the config's size, acquire
timeout and lifetimes and sends the three session timeouts as startup parameters:

| Function | Use |
| --- | --- |
| `connect_with_backoff(&config, budget)` | API and worker startup. Retries transient errors within `budget`, fails fast on auth and configuration errors |
| `connect_for_migrations(&config, budget)` | the migrate binary. Sends only `lock_timeout`, so a long migration is not cut off |
| `connect_pool(&config)` | one attempt, no retry |
| `connect_pool_lazy(&config)` | opens connections on first use |
| `pool_options(&config)`, `connect_options(&config)` | the two halves, for a custom pool |

`connect_with_backoff` and `connect_for_migrations` return `StartupError<sqlx::Error>`, re-exported
from `baukit_resilience::startup` together with `RetryAttempt`. `is_transient_connect_error` exposes the
classification.

```rust
let pool = baukit_ops::connect_with_backoff(database, config.startup.budget).await?;
```

A worker whose jobs run longer than the request timeout raises `statement_timeout` on its own copy of
the config, as the worker template does with `worker.job_timeout_seconds`.

## Redis

`RedisRateLimitStore::connect_if_enabled(&options, startup_budget)` takes the budget and returns
`StartupError<RateLimitStoreError>`. When Redis is still down after the budget it starts with a
degraded store that reconnects in the background. `is_connected()` reports which state it is in.
`connect_with_backoff` and `connect_or_degraded` are the building blocks. `from_connection_manager` is no
longer `const`.

## Identity provider

`OidcVerifier::discover` and `BrowserSessionState::discover` retry transient discovery failures for 60 s
by default. Change it with `OidcConfig::with_startup_budget` or
`BrowserSessionConfig::with_startup_budget`. [fail-closed-503-and-grace.md](fail-closed-503-and-grace.md)
lists the new error variants.

## Shared platform Postgres

PgBouncer in transaction mode drops the client's startup parameters, so on shared Postgres the session
timeouts live on the owner role. The bootstrap owner `product_owner` gets them through
`postInitApplicationSQL`, which CNPG runs only when it creates the cluster. On an existing cluster, or
for any other tenant owner, have the owner run the statements on itself once over a direct connection to
`postgres-cluster-rw`:

```bash
psql "host=postgres-cluster-rw.postgres dbname=product user=product_owner sslmode=require" \
  -c "ALTER ROLE product_owner SET statement_timeout = '29s'" \
  -c "ALTER ROLE product_owner SET lock_timeout = '5s'" \
  -c "ALTER ROLE product_owner SET idle_in_transaction_session_timeout = '60s'"
```

Set `connectionLimit: 25` on the owner's `DatabaseRole`, as
`deploy/platform/postgres-cluster/product-database.example.yaml` shows. The pooler takes at most 12 of
them (`max_db_connections`), which leaves 13 direct connections for one worker pod, migrations and an
operator.

Only the API should go through the pooler. In the `baukit-app` chart, set
`backing.postgres.shared.directUrlSecretRef` to a Secret with a URL to `<clusterName>-rw`. The worker,
migrate and seed processes then use it, keep their own `statement_timeout`, and are not cut off by the
pooler's 35 s `query_timeout`. `backing.postgres.shared.poolerName` names the pooler the API may reach
and defaults to `<clusterName>-pooler-rw`.
