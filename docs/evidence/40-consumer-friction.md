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

The observations below record the product pins at the time of the survey.
Baukit now defaults to `postgres:18.6-alpine`, and products should adopt
PostgreSQL 18 instead of retaining these older overrides.

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
  The default is now `postgres:18.6-alpine`.
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

Docker tests in `postgres.rs`: an overridden `18.6-alpine3.23` tag, an app role bound
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
  `finops-test` with `PostgresTestOptions::new().with_app_role(...)` and use a
  product-pinned PostgreSQL 18 TimescaleDB image if extensions are required. Use
  `app_connection_url()` for `app_pool`.
- Hebkit: replace the hand-built container in `tests/common/mod.rs:186-206`
  with `PostgresTestOptions::new()` and its PostgreSQL 18.6 default.
- Redemut: replace the per-test `CREATE DATABASE` code in
  `tests/support/mod.rs:242-360` with `create_database()`.
- Solo Leveling System: replace the `DATABASE_URL` branch with
  `PostgresTestDatabases`.

### Product defect

Solo Leveling System's `DATABASE_URL` branch never drops `sl_test_<uuid>` and
returns the admin `database_url` instead of the per-test database URL, so those
tests share one database while believing they are isolated.

## 3. Analytics scrubber exact keys and crash events

### Observed repeated glue

`scrubProperties` matched blocked keys only as substrings. A short key like `ip`
would redact `zip_code` and `description`, so products added their own exact
matching, and crash reports went through separate scrubbers:

- `/home/patrick/projects/hebkit/mobile/src/monitoring/scrub.ts` runs an exact
  key regex (`ip`, `q`, `query`, `notes`, `remote_addr`, `x-forwarded-for`, and
  more) before `scrubProperties`, and uses the result for Sentry too. That path
  also sends event and trace IDs through the long-hex rule, which redacts them,
  and `filename` through the `name` rule, which redacts stack frame files.
- `/home/patrick/projects/tiefgang/mobile/src/monitoring/sentry.ts` has its own
  substring regex scrubber for Sentry `beforeSend`, with no value patterns.

### Baukit owner and public contract

`typescript/packages/analytics-core/src/scrubber.ts`:

- `ScrubberOptions.exactBlockedKeys` and `AnalyticsClientOptions.exactBlockedKeys`.
  Keys are normalized like `blockedKeys` (lowercase, separators removed) and
  compared by equality.
- `DEFAULT_EXACT_BLOCKED_KEYS`: `ip`, `ip_address`, `remote_addr`,
  `x_forwarded_for`, `x_real_ip`.
- `scrubErrorEvent(event, options)` for crash reports. It adds
  `ERROR_EVENT_BLOCKED_KEYS` (`headers`, `data`, `query_string`, `body`, `vars`,
  `geo`, `env`) and keeps string values under `ERROR_EVENT_PRESERVED_KEYS`
  (event, trace, span, and debug IDs, and frame `filename`, `abs_path`,
  `function`, `module`). The top-level `sdk` object keeps its keys and only has
  its values checked.

### Failure behavior

The scrubber never throws. Values that are not plain objects, arrays, or
primitives still become `[redacted]`, including under preserved keys.

### Privacy boundary

Preserved keys keep only string values. A preserved key holding an object is
scrubbed like any other value. OS and device `name` keys stay redacted because
the scrubber cannot tell them apart.

### Supported runtimes

Unchanged: ES2022 runtimes, React Native Hermes, and Node 24.

### Tests

`scrubber.test.ts` covers exact keys next to longer keys that contain them, a
Sentry-shaped event with IDs, frames, request payloads, breadcrumbs, user, sdk,
and extra, input immutability, preserved keys holding objects, and product
extensions. `client.test.ts` checks that the client passes `exactBlockedKeys`.

### Breaks

`scrubProperties` and `AnalyticsClient` now redact `ip`, `ip_address`,
`remote_addr`, `x_forwarded_for`, and `x_real_ip` by default. Recorded in the
changeset.

### Product adoption

- Hebkit: move the exact names from `EXACT_PRODUCT_KEY_PATTERN` into
  `exactBlockedKeys`, drop `redactExactProductKeys`, and use `scrubErrorEvent`
  in Sentry `beforeSend` instead of `scrubPii`.
- Tiefgang: replace `scrubSentryEvent` in `mobile/src/monitoring/sentry.ts`
  with `scrubErrorEvent(event, { blockedKeys: ['intention', 'note', 'panic',
  'request', 'target'] })`.

## 4. Jest resolves `@baukit/*` without a module map

### Observed repeated glue

Every package export listed only `types` and `import`. Jest resolves from
CommonJS with the `require` and `default` conditions plus its environment
conditions, so it found no match and every mobile app kept a hand-written
`moduleNameMapper` that points each package at `node_modules/.../dist/*.js`:

- Baukit's own `templates/mobile/mobile/jest.config.cjs` (eight entries) and
  `templates/mobile/__auth__/mobile/jest.config.cjs` (ten entries).
- `/home/patrick/projects/eigenruhe/mobile/jest.config.cjs:45`,
  `/home/patrick/projects/hebkit/mobile/jest.config.cjs:5`,
  `/home/patrick/projects/leitbild/mobile/jest.config.cjs:7`,
  `/home/patrick/projects/redemut/mobile/jest.config.cjs:5`,
  `/home/patrick/projects/schlauzug/mobile/jest.config.cjs:7`,
  `/home/patrick/projects/solo-leveling-system/mobile/jest.config.cjs:6`, and
  `/home/patrick/projects/tiefgang/mobile/jest.config.cjs:9`.
- `/home/patrick/projects/hebkit/mobile/package.json:117` repeats a larger map,
  including `sync-client` subpaths, inline in its `test` script.

Each new Baukit package or subpath broke those tests until someone extended the
map.

### Decision: `default` condition, not a Jest preset

Every export except the `./vitest` subpaths now lists `default` with the same
target as `import`. The build output is ESM, and Jest already transforms it
because the template sets `transformIgnorePatterns: []`, so pointing `default`
at the ESM file is enough. It also fixes any other resolver that uses CommonJS
conditions, needs no new package, and cannot fall out of date the way a preset's
map would when a subpath is added. A preset would have needed its own package,
version, and a map that repeats the exports field.

`@baukit/pwa-web` is unchanged: it already publishes `require` with a CommonJS
build. The `./vitest` subpaths of `data-contracts` and `preferences-core` stay
`import`-only because Vitest is ESM-only and must not load under Jest.

### Failure behavior

A Jest suite that imports a `./vitest` subpath fails resolution with
`ERR_PACKAGE_PATH_NOT_EXPORTED` instead of failing inside Vitest.

### Privacy boundary

None.

### Supported runtimes

Jest 29 through `jest-expo`, Node 24, Vite, and Metro. Metro and Vite already
used `import`.

### Tests

`typescript/scripts/test-packed-exports.mjs` runs at the end of every package's
`test` script except `pwa-web`, which has its own packed test. It packs the
package with pnpm, extracts the archive into a temporary `node_modules`, and
resolves every export with Node's `require` conditions. It fails when an export
does not resolve or points at a file missing from the archive, and it checks
that each `--esm-only` subpath still refuses `require`. The script failed on
`@baukit/analytics-core` before the `default` condition was added. The
generated `--mobile` fixture and the `--auth oidc` fixture pass `tsc --noEmit`,
lint, and Jest with the maps removed.

### Template change

Both mobile `jest.config.cjs` files drop `moduleNameMapper` and keep
`transformIgnorePatterns: []`. The mobile README explains why no map is
needed.

### Breaks

None for consumers. Existing maps keep working because they bypass export
resolution.

### Product adoption

- Eigenruhe, Hebkit, Leitbild, Redemut, Schlauzug, Solo Leveling System, and
  Tiefgang: delete the `@baukit/*` entries from `mobile/jest.config.cjs` and keep
  `transformIgnorePatterns` covering `@baukit`.
- Hebkit: also delete the `@baukit/*` entries from the inline
  `--moduleNameMapper` in `mobile/package.json:117`.

## 5. `MockOidcServer` JWKS cache helpers

### Observed repeated glue

`/home/patrick/projects/hebkit/backend/tests/common/mod.rs` keeps its own
`FakeOidc` server next to Baukit's `MockOidcServer`. It exposes a `jwks_url`, a
JWKS request counter, `start_with_jwks_delay`, and `token_with(..., key_id)`.
`backend/tests/auth_conformance.rs` uses them for TTL refresh, negative caching
of an unknown `kid`, and single-flight refresh under a 75 ms JWKS delay, with a
verifier built through `OidcVerifier::from_jwks_uri`.

### Correction to the plan's premise

The plan asked for a JWKS request counter and an injectable JWKS delay. Both
already existed as `MockOidcServer::jwks_request_count` and `set_jwks_delay`
since 0.1.0, with tests, but the README never mentioned them. What Hebkit could
not get from Baukit was the JWKS URL for a verifier that skips discovery, and a
token with an unpublished `kid`: Baukit's own unknown-key test used the private
key file directly.

### Baukit owner and public contract

`rust/crates/baukit-test/src/jwt.rs`:

- `MockOidcServer::jwks_url() -> &str`.
- `MockOidcServer::mint_with_key_id(&claims, key_id)`: signs with the active
  key, so the signature matches a published key while the `kid` does not.
- The README now documents `jwks_request_count` and `set_jwks_delay` together
  with the two additions.

### Failure behavior

Unchanged. `mint_with_key_id` returns `JwtFixtureError` like `mint`.

### Privacy boundary

Test-only fixture keys. No change.

### Supported runtimes

Tokio, as before.

### Tests

`jwt.rs` gains a TTL refresh test through `jwks_url` and `from_jwks_uri`, and a
test that `mint_with_key_id` with the published `kid` equals `mint` before and
after rotation. The existing single-flight and negative-cache test now uses
`mint_with_key_id` instead of the private key.

### Breaks

None.

### Product adoption

- Hebkit: replace `FakeOidc` in `backend/tests/common/mod.rs` with
  `MockOidcServer`. Use `jwks_url()` for `settings(...)` and
  `from_jwks_uri`, `set_jwks_delay` after `start()` instead of
  `start_with_jwks_delay`, and `mint_with_key_id` instead of `token_with`. Its
  claims must use `server.issuer()`.

## 6. Bearer challenges use `error_description`

### Observed repeated glue

`AuthRejection` answered a rejected token with
`Bearer error="invalid_token", hint="expired"` or `hint="invalid"`. RFC 6750
defines `error_description` for this and has no `hint` parameter.
`/home/patrick/projects/hebkit/backend/crates/hebkit-api/src/adapters/http/error.rs:553-567`
rebuilds the challenge itself to send `error_description="expired"` and
`error_description="invalid"`.

### Baukit owner and public contract

`rust/crates/baukit-auth/src/axum_integration.rs` sends:

- `Bearer error="invalid_token", error_description="expired"` for
  `AuthRejection::ExpiredToken`.
- `Bearer error="invalid_token", error_description="invalid"` for
  `AuthRejection::InvalidToken`, which covers every other verification failure.
- A bare `Bearer` challenge for a request without credentials, as before.

The values match Hebkit's, so its clients need no change. Both headers are
static strings, so no claim value, issuer, or verifier error text can reach
them.

### Failure behavior

Unchanged status and body: `401` with the `unauthenticated` envelope.

### Privacy boundary

The header carries one of two fixed words. The README states this.

### Supported runtimes

Any HTTP client. No Baukit TypeScript package parsed `hint`.

### Tests

The existing challenge tests in `axum_integration.rs` and `api_token.rs`, the
`check_auth_router_conformance` check in `baukit-test`, and the generated
`backend/tests/auth_conformance.rs` now pin the new header. The `--auth oidc`
fixture backend passes fmt, clippy, and all tests including ignored ones.

### Template change

`templates/backend/__auth__/backend/tests/auth_conformance.rs` expects the new
header.

### Breaks

- `baukit-auth`: the `hint` parameter is gone. A client that read it must read
  `error_description`.
- `baukit-test`: `check_auth_router_conformance` fails a router that still sends
  `hint`.

### Product adoption

- Hebkit: drop the hand-built challenge in
  `backend/crates/hebkit-api/src/adapters/http/error.rs:553-567` and return
  `AuthRejection` instead.
