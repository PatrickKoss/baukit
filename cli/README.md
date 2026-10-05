# Baukit CLI

Install the CLI from the release tag:

```sh
cargo install --git https://github.com/PatrickKoss/baukit --tag v0.7.2 --locked baukit-cli
baukit --version
```

The CLI is not published to crates.io. Use the tag that matches the product's
`template_version` so its embedded templates and dependency versions agree.

`baukit new NAME ...` creates a new `NAME/` directory. To scaffold an existing
or orphan-branch repository root without overwriting differing files, use
`--dir . --into-existing`. Generation deliberately does not commit or push:

```sh
git add .
git commit -m "Scaffold product with Baukit"
git remote add origin git@github.com:YOUR_ORG/NAME.git
git push -u origin main
```

Use `--port-offset N` when several generated products run on one development
machine. The CLI adds `N` to each generated PostgreSQL, API, operations,
Keycloak, and fake-provider host port, then records the offset in `baukit.toml`.
An offset of zero keeps the default ports and is omitted from the manifest.
`baukit doctor` checks the generated port references against the recorded
offset.

`openapi.consumers` lists generated TypeScript declarations. Raw schema copies
remain product-owned until a second product needs them. The
[raw OpenAPI mirror design](../docs/platform/openapi-mirrors.md) records the
proposed manifest and strict-check behavior.

Each frontend can have its own `pnpm-workspace.yaml`. A product can instead
use one root workspace whose `packages` patterns include every enabled app
(`mobile`, `web`, and pnpm-managed `mcp`). Doctor checks inclusions and exclusions in those
patterns.

`capabilities.analytics` selects the mobile analytics adapter. It accepts
`"posthog"` or `"none"`. Generated manifests select `"posthog"`; older
manifests without this setting do not select an adapter. Set it to `"none"`
when the product does not collect analytics. Doctor requires
`@baukit/analytics-posthog-native`
only for `"posthog"`. With `"none"`, it requires neither analytics files nor
`@baukit/analytics-core`. The web template uses `NoopTransport` and has no required
PostHog adapter. Auth adapters follow `capabilities.auth`; the Expo SQLite
adapter remains required by the generated mobile record store.

Products with independently assigned ports can override the offset in
`baukit.toml`. Host ports apply to local URLs and `make dev`; container ports
apply to Compose targets and deployment values. For example:

```toml
[ports]
api = { host = 17001, container = 8080, service = "backend" }
ops = { host = 17002, container = 9090, service = "backend" }
postgres = { host = 17003, container = 5432 }
keycloak = { host = 17004, container = 8080 }
```

The other supported names are `redis` and `fake_provider`. `service` overrides
the Compose service name. Undeclared ports retain their offset defaults.
PostgreSQL, Keycloak, and Redis must keep container ports 5432, 8080, and 6379
for the generated images. Doctor checks literal Compose mappings and parameter
defaults against declarations, and checks loopback source URLs against host
ports. Runtime environment overrides remain product-owned.

Doctor checks identity where code consumes it. Analytics context names and MCP
server names must match `app.name`. It follows local constants and relative
named imports, including renamed bindings. Expo's slug can use a product's
public name. Backend config consumers must agree on one namespace; authenticated
backends use `app.name`. An unauthenticated backend can use a different config
namespace, which its analytics context can also use. For example, SLS uses
`sl` for packages and `solo-leveling-system` for config and analytics.

A product needs a consumed identity source. Doctor accepts inline Expo config,
MCP server metadata, and backend `ConfigLoader::new` calls. It does not require
`src/product.ts` or a library constant named `PRODUCT`. Missing imported bindings,
empty identities, and conflicting backend namespaces still fail. These static
checks cover literal names and constants; compilation checks computed values.

Doctor does not require the template's guidance filenames. Products can write
API policy, fake-provider, sync-table, navigation, observability, budget, and
local-data-retention guidance under their own names. Generated links should
be updated when a document moves. Machine-read inputs retain their contracts:
`mcp/docs/tools.md` for `docs:check`, the declared OpenAPI schema and consumers,
and `docs/openapi-accepted-breaks.json` when the compatibility gate reads it.

MCP drift checks and tests read the exports in `mcp/src/tools/read.ts` and
`write.ts`, so those files stay required. Doctor accepts exported `READ_TOOLS`
and `WRITE_TOOLS` metadata, or `READ_TOOL_NAMES` and `WRITE_TOOL_NAMES` catalogs.
The shared helper `mcp/src/tools/registry.ts` is a template implementation
choice. A product can move its types and result helpers if it updates its own
imports and keeps its tool docs, route checks, and server tests passing.

## Doctor paths for existing products

Doctor reads the Cargo workspace declared by the product. It scans those crates
for limits, worker and authentication wiring, including their declared test
and binary targets. Crate names do not need to match `app.name`. Mobile auth
checks use source symbols, so files can move or have product-owned names.
Doctor does not require a Keycloak guidance filename.

Paths below are optional and relative to the product root. They cannot contain
`..`. `backend_manifest` defaults to `backend/Cargo.toml`; `migrations` defaults
to `backend/migrations`. Dockerfile and .dockerignore default to the backend
manifest directory. Keycloak inputs and tools are discovered by their
content. Doctor reports ambiguous inputs instead of choosing one.

```toml
[doctor]
backend_manifest = "backend/Cargo.toml"
backend_dockerfile = "backend/Dockerfile"
backend_dockerignore = "backend/.dockerignore"
migrations = "database/migrations"
keycloak_realm = "docker/keycloak/realm-export.json"
keycloak_policy = "docker/keycloak/realm-policy.json"
keycloak_reconcile = "docker/keycloak/reconcile.json"
keycloak_policy_tool = "tools/keycloak_policy.py"
keycloak_reconcile_tool = "tools/reconcile_keycloak.py"

[doctor.sources]
backend_limits = "backend/crates/sl-domain/src/policy.rs"
worker_entry = "backend/crates/sl-bin/src/bin/jobs.rs"
worker_tests = "backend/crates/sl-bin/tests/jobs.rs"
auth_tests = "backend/crates/sl-api/tests/oidc.rs"
pkce_login = "tools/login.py"
mobile_sign_in = "mobile/app/(auth)/login.tsx"
mobile_auth = "mobile/src/services/session.ts"
mobile_auth_tests = "mobile/src/services/session.test.ts"
mobile_local_data = "mobile/src/storage/partition.ts"
mobile_persistence = "mobile/src/storage/identity.ts"
keycloak_policy_tests = "tools/tests/policy_test.py"
keycloak_reconcile_tests = "tools/tests/reconcile_test.py"
```

A source override limits the check to that file. The file must still contain
the checked wiring; an existing empty file does not pass. Without an override,
Doctor looks for these symbols:

| Source key | Required content |
| --- | --- |
| `backend_limits` | A shared limits check such as `check_measurement`, or a measurement such as `trimmed_unicode_scalar_count` used by product validation |
| `worker_entry` | A `main` function using `ProcessKind::Worker` |
| `worker_tests` | `WorkerRunner` or a PostgreSQL test database fixture |
| `auth_tests` | `check_auth_router_conformance` or `MockOidcServer` |
| `pkce_login` | Optional developer login helper. Check `code_challenge` and `S256` when a path is declared. |
| `mobile_sign_in` | `signIn`, `signInWithOidc` or `login` in an app route or its re-exported screen |
| `mobile_auth` | `createExpoOidcClient`, `createNativeOidcClient` or `NativeOidcClient` |
| `mobile_auth_tests` | `signIn` or `signInWithOidc` in a test |
| `mobile_local_data` | `ScopedPersistenceRegistryStore` |
| `mobile_persistence` | `ScopedPersistenceLifecycle` |
| `keycloak_policy_tests` | unittest coverage of `validate_realm` |
| `keycloak_reconcile_tests` | unittest coverage of `load_reconcile_config`, `validate_inputs` or `RealmReconciler` |

Doctor requires the local Redis URL when backend source uses a Redis-backed
adapter. OIDC alone does not require Redis. PostgreSQL-backed rate limiting can
omit that URL.
