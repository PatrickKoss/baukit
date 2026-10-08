# Baukit CLI

Install the CLI from the release tag:

```sh
cargo install --git https://github.com/PatrickKoss/baukit --tag v0.10.2 --locked baukit-cli
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

Use `--pwa` with `--mobile` or `--web` to generate the service worker build.
The web app hosts it when both apps are present. Otherwise Expo web hosts it.
Run `corepack pnpm run build:sw` before exporting the Expo web app, then
`corepack pnpm run build:sw:check` to check the worker artifact.

Use `--auth oidc`, `--auth clerk` or `--auth workos` with any backend, web or
mobile flavor. Each also supports `--backend --mcp`. Only OIDC includes the
development Keycloak realm. Managed providers use their official web SDKs;
mobile uses Clerk Expo or WorkOS AuthKit public-client PKCE. Generated READMEs
describe client IDs, publishable keys and backend secret bindings.
The generated backend's `docs/auth-providers.md` explains external OIDC issuers.
See [remote MCP](../docs/remote-mcp.md) for each provider's OAuth client setup
and token binding.

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
(`mobile` and `web`). Doctor checks inclusions and exclusions in those
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

Doctor checks identity where code consumes it. Analytics context names must match `app.name`. It follows local constants and relative
named imports, including renamed bindings. Expo's slug can use a product's
public name. Backend config consumers must agree on one namespace; authenticated
backends use `app.name`. An unauthenticated backend can use a different config
namespace, which its analytics context can also use. For example, SLS uses
`sl` for packages and `solo-leveling-system` for config and analytics.

A product needs a consumed identity source. Doctor accepts inline Expo config
and backend `ConfigLoader::new` calls. It does not require
`src/product.ts` or a library constant named `PRODUCT`. Missing imported bindings,
empty identities, and conflicting backend namespaces still fail. These static
checks cover literal names and constants; compilation checks computed values.

Doctor does not require the template's guidance filenames. Products can write
API policy, fake-provider, sync-table, navigation, observability, budget, and
local-data-retention guidance under their own names. Generated links should
be updated when a document moves. Doctor checks the declared OpenAPI schema and
consumers. For MCP, it checks the Rust router, OAuth configuration, metadata
route, scoped tool registration and schema drift test. A retired TypeScript
server or MCP capability table receives a migration finding. Follow the
[MCP migration guide](../docs/migrations/mcp-stdio-to-remote.md).

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

Doctor scans MCP wiring in the backend's declared Cargo packages, including
split router, configuration and tool-definition modules. Override individual
checks under `[doctor.sources]` with `mcp_router`, `mcp_config`, `mcp_tools`, or
`mcp_drift`. Each value is a path relative to the product root. Doctor checks
symbols in those files and continues to report missing wiring.
