# 30. Consumer friction in tooling and test support

Plan item 4. Six small corrections that products patch around today. Each one
lands as its own commit with its own test and changelog line.

## Source revisions

Product files were re-read on 2026-09-27 at Eigenruhe `f74cebb`, Hebkit
`841bf5d`, Redemut `a782538`, Runtime Analyzer `d47bfd5`, Solo Leveling System
`3461eaf`, and Tiefgang `2d37a06`. Eigenruhe moved past the plan's `e44ff88`;
its lint script did not change in a way that affects this note. Baukit baseline
is `ef6ac1a`.

## 1. Metric-name linter takes product input

### Observed repeated glue

`deploy/observability/lint/check-metric-names.py` hardcoded its root and metric
list. Every product that lints dashboards loads the file as a module and patches
its globals:

- `/home/patrick/projects/redemut/scripts/observability-lint.py:38-56` patches
  `ROOT` and `SPEC_METRICS`, then builds a temporary symlink tree because its
  dashboards live in `deploy/observability/grafana/dashboards` and its alerts in
  a single `deploy/observability/alerts.yml`.
- `/home/patrick/projects/tiefgang/infra/observability-lint.py:47-50` patches
  `ROOT`, `OBSERVABILITY` (`infra/grafana`), and `SPEC_METRICS`.
- `/home/patrick/projects/eigenruhe/scripts/observability-lint.py:128-133`
  patches the same globals, then runs its own dashboard coverage audit.
- `/home/patrick/projects/runtime-analyzer/deploy/observability/lint/check-metric-names.py:31-45`
  is an edited vendored copy that adds seven product names to `SPEC_METRICS`
  and lists `job_duration_seconds` a second time in `HISTOGRAM_METRICS`.
- The generated `docs/observability-lint.md` documented the module-global patch
  as the contract.

### Baukit owner and public contract

`deploy/observability/lint/check-metric-names.py` gains three arguments:

- `--observability-root DIR`: directory holding `dashboards/`, `alerts/`, and
  `recording-rules/`. Defaults to Baukit's own `deploy/observability`.
- `--allowlist FILE`: one product metric per line, `#` comments, and an optional
  `histogram` marker that also allows `_bucket`, `_count`, and `_sum`.
- `--rules FILE`: extra rule file outside the root, repeatable. Redemut's
  single `alerts.yml` needs it.

`main(argv)` takes the argument list. Called with no arguments, as CI and
`verify/verify-observability.sh` do, it lints Baukit's pack and prints the same
summary line as before. Paths in messages are relative to the Baukit checkout
when inside it, otherwise relative to the working directory.

### Failure behavior

Exit 0 on success and 1 on lint problems, as before. Exit 2 when the root is not
a directory, the allowlist cannot be read, or an allowlist line has an invalid
name, an unknown marker, or a duplicate name. A missing root used to pass with
zero files, which hid a typo in a product path.

### Privacy boundary

None. The linter reads only local dashboard and rule files.

### Supported runtimes

Python 3.9 or later, standard library only.

### Tests

`deploy/observability/lint/test_check_metric_names.py` covers the unchanged
Baukit invocation, a product root with an allowlist and histogram series,
missing allowlist entries, the histogram marker, `--rules`, invalid allowlist
entries, and a missing root. CI runs it next to the linter. The new arguments
were also run by hand against Redemut, Tiefgang, and Runtime Analyzer at the
revisions above; all three passed with an allowlist built from their current
product names.

### Template change

The generated CI job and the strict `scripts/quality-gate.sh` now look for
`deploy/observability/product-metrics.txt` instead of
`scripts/observability-lint.py` and call the linter with
`--observability-root deploy/observability --allowlist
deploy/observability/product-metrics.txt`. The generated
`docs/observability-lint.md` documents the allowlist instead of the shim.

### Breaks

- A generated product's CI no longer runs `scripts/observability-lint.py`. A
  product that relies on its shim must add
  `deploy/observability/product-metrics.txt` and adjust the arguments in its CI
  job to its layout.
- Shims that set module globals and call `linter.main()` still work, because
  the defaults read those globals at call time. That path is no longer
  documented.

### Product adoption

- Redemut: delete `scripts/observability-lint.py`; call the linter with
  `--observability-root deploy/observability/grafana --rules
  deploy/observability/alerts.yml --allowlist <file>` and mark
  `redemut_sync_batch_size` as `histogram`.
- Tiefgang: delete `infra/observability-lint.py`; call the linter with
  `--observability-root infra/grafana --allowlist <file>`.
- Eigenruhe: keep the product coverage audit in `scripts/observability-lint.py`,
  but replace the global patching at `:128-133` with a call to
  `linter.main(["--observability-root", "infra/grafana", "--allowlist", ...])`
  or write the allowlist from `product_metrics.rs` first.
- Runtime Analyzer: delete the vendored
  `deploy/observability/lint/check-metric-names.py`, move its seven product
  names to an allowlist with `job_duration_seconds histogram`, and run Baukit's
  copy with `--observability-root deploy/observability`.

## 2. PostgreSQL test container options

### Observed repeated glue

`baukit_test::start_postgres` pinned `postgres:18-alpine`, connected only as the
`postgres` superuser, and gave one database per container. Products work around
each limit:

- `/home/patrick/projects/runtime-analyzer/backend/crates/finops-test/src/lib.rs`
  starts `timescale/timescaledb:latest-pg17` as a `GenericImage` with its own
  query-probe readiness loop, then builds an `app_pool` as `finops_app`.
  Superuser connections bypass row-level security, so RLS tests need that role.
  Its migrations create the role with `IF NOT EXISTS` and manage their own
  grants and revokes.
- `/home/patrick/projects/hebkit/tests/common/mod.rs:186-206` starts
  `postgres:16-alpine` itself.
- `/home/patrick/projects/redemut/tests/support/mod.rs:242-360` starts
  `postgres:17-alpine` once and runs `CREATE DATABASE test_<uuid>` per test on
  the shared container.
- `/home/patrick/projects/solo-leveling-system/backend/crates/sl-api/tests/common/mod.rs:44-81`
  has a `DATABASE_URL` branch that creates `sl_test_<uuid>` and never drops it.

### Baukit owner and public contract

`rust/crates/baukit-test/src/postgres.rs` and `postgres_database.rs`:

- `PostgresTestOptions::new()` with `with_image(name, tag)`,
  `with_app_role(PostgresAppRole)`, `with_migrations(path)`, and `start()`.
  The default is the old `postgres:18-alpine` fixture.
- `PostgresAppRole::new(name, password)` validates both. The role is created
  `LOGIN NOSUPERUSER NOBYPASSRLS` before migrations, gets `USAGE` on `public`,
  and default privileges for tables and sequences that migrations create.
  Migrations run later, so their own `REVOKE` statements still win.
- `PostgresTestContainer::app_connection_url()` and `create_database()`.
- `PostgresTestDatabases::new(admin_url)` with the same `with_app_role` and
  `with_migrations`, and `create()` for a server the test does not own.
- `PostgresTestDatabase` exposes `name()`, `connection_url()`,
  `app_connection_url()`, and `drop_database()`. Its `Drop` runs
  `DROP DATABASE IF EXISTS ... WITH (FORCE)` on a helper thread.

`start_postgres()` and `start_postgres_with_migrations()` keep their
signatures, so the callers in `baukit-jobs`, `baukit-sync`, the inbox and
live-row-cap checks, and the backend and worker templates compile unchanged.

### Failure behavior

`PostgresTestError::InvalidAppRole` for a name outside `[a-z_][a-z0-9_]*`, a
password outside the URL-unreserved set, or an existing role that is a superuser
or has `BYPASSRLS`. `InvalidConnectionUrl` when the admin URL cannot be parsed;
the message leaves the URL out. `Setup` wraps a failed `CREATE ROLE`,
`CREATE DATABASE`, grant, or drop. Readiness waits up to 60 seconds for the
first connection.

### Privacy boundary

Test-only. Debug output of `PostgresAppRole`, `PostgresTestDatabases`, and
`PostgresTestDatabase` hides passwords and connection URLs. An existing role's
password is never changed.

### Supported runtimes

PostgreSQL 13 or later for `DROP DATABASE ... WITH (FORCE)`. Any image that
accepts the official `POSTGRES_USER`, `POSTGRES_PASSWORD`, and `POSTGRES_DB`
variables and prints the official readiness lines.

### Tests

Docker tests in `postgres.rs`: an overridden `17-alpine` tag, an app role bound
by an RLS policy for reads and writes and reporting `rolsuper` and
`rolbypassrls` false, two per-test databases dropped explicitly and by `Drop`
while a pool is still open, an external-server database dropped by `Drop`, and
the `postgres` superuser rejected as an app role. Unit tests in
`postgres_database.rs` cover name and password validation, redacted Debug
output, and URL rewriting.

### Breaks

`PostgresTestError` gains three variants. An exhaustive `match` on it no longer
compiles. With `sqlx-postgres`, `start_postgres` opens one connection before it
returns.

### Product adoption

- Runtime Analyzer: replace the `GenericImage` and readiness loop in
  `finops-test` with `PostgresTestOptions::new().with_image("timescale/timescaledb",
  "latest-pg17").with_app_role(PostgresAppRole::new("finops_app", ...))` and use
  `app_connection_url()` for `app_pool`.
- Hebkit: replace the hand-built container in `tests/common/mod.rs:186-206`
  with `PostgresTestOptions::new().with_image("postgres", "16-alpine")`.
- Redemut: replace the per-test `CREATE DATABASE` code in
  `tests/support/mod.rs:242-360` with `create_database()`.
- Solo Leveling System: replace the `DATABASE_URL` branch with
  `PostgresTestDatabases`.

### Product defect

Solo Leveling System's `DATABASE_URL` branch never drops `sl_test_<uuid>` and
returns the admin `database_url` instead of the per-test database URL, so those
tests share one database while believing they are isolated.
