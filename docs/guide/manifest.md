# Configure a product with baukit.toml

`baukit.toml` at the product root records what `baukit new` generated and how the product wants
its gate, ports, images and deployment set up. This page lists every section and key that
`cli/src/manifest.rs` parses, with its type, its default and what reads it. Read it before you edit
the manifest by hand.

Three readers use the file:

- `baukit new` reads an existing manifest when you re-render with `--force` or `--into-existing`.
- `baukit doctor` and `baukit generate` read it on every run.
- The strict gate runner, `scripts/gate.py`, reads the `[quality]` and `[gate]` keys it needs.

`baukit new` writes the manifest once. Later renders keep it byte for byte, so changes after
generation are yours to make. Run `baukit doctor` after each edit.

## A generated manifest

`baukit new docscheck --backend --web --quality strict` writes:

```toml
schema_version = 1
template_version = "0.31.0"

[app]
name = "docscheck"

[quality]
profile = "strict"
backend_coverage_lines = 70
critical_paths = []
webkit_repeats = 3
full_stack_e2e = false
openapi_compatibility = "off"
shared = []

[quality.gate_budget_minutes]
full = 10
affected = 5

[quality.affected]
"backend/crates/**" = ["backend", "fuzz", "images"]
"web/src/**" = ["web"]
"docs/**" = []

[capabilities]
analytics = "posthog"
backend = true
worker = false
mobile = false
web = true
pwa = false

[dependencies.baukit]
source = "registry"
version = "0.31.0"

[openapi]
schema = "backend/openapi.json"
consumers = ["generated/openapi.d.ts"]

[gitops]
domain = "lab.example.com"
registry = "ghcr.io/example"
```

The real `[quality.affected]` table has more globs. The templates are `templates/*/baukit.toml`,
and `cli/src/generate/manifest_source.rs` merges them.

## Top-level keys

| Key | Type | Default | Read by |
| --- | --- | --- | --- |
| `schema_version` | integer | required | doctor, which fails unless it is `1`. |
| `template_version` | string | required | doctor, which fails unless it equals the CLI version. |
| `port_offset` | integer | `0` | `new` and doctor. Added to the default service ports. `new` omits it when 0. |

## `[app]`

| Key | Type | Default | Read by |
| --- | --- | --- | --- |
| `name` | string | required | Every command. Crate names, env prefixes, image names and the realm come from it. |

## `[capabilities]`

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `backend`, `mobile`, `web` | bool | required | The generated apps. |
| `worker`, `pwa`, `mcp`, `platform` | bool | `false` | The matching `baukit new` flags. |
| `auth` | `"oidc"`, `"clerk"` or `"workos"` | unset | The auth provider. |
| `analytics` | `"posthog"` or `"none"` | `"none"` | `"posthog"` makes doctor require `@baukit/analytics-posthog-native` in mobile. New products get `"posthog"`. |

Doctor rejects the same combinations `baukit new` does, such as `mcp` without `auth`.

## `[dependencies.baukit]`

Where the product gets baukit crates and packages. `source` picks the shape:

| `source` | Other keys | Written by |
| --- | --- | --- |
| `"registry"` | `version` | `baukit new` without `--baukit-path`. Doctor checks every crates.io and npm pin against `version`. |
| `"path"` | `path` | `baukit new --baukit-path`. Doctor checks that `path` holds `crates/baukit-config`. |
| `"git"` | `git`, `tag` | Older products only. Doctor checks the SSH agent and the tag. |

## `[openapi]`

| Key | Type | Default | Read by |
| --- | --- | --- | --- |
| `schema` | string | required | doctor, `generate openapi-client`, the gate. The committed OpenAPI document. |
| `consumers` | list of strings | `[]` | `generate openapi-client`, `scripts/openapi-client.sh`, the gate. Each committed TypeScript declaration. |
| `typescript` | string | unset | Older single-consumer form, used when `consumers` is empty. |

## `[quality]`

`profile` and the thresholds apply to both profiles. The gate keys apply only in strict products.
The [quality gates guide](quality-gates.md) explains how the gate uses them.

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `profile` | `"standard"` or `"strict"` | `"standard"` | Set by `--quality`. |
| `backend_coverage_lines` | integer 1 to 100 | `70` | Line coverage floor of the backend tests. |
| `critical_paths` | list of strings | `[]` | `e2e/tests/*.spec.ts` paths under `web/`, repeated on WebKit. Needs `web`. |
| `webkit_repeats` | integer above 0 | `3` | Repeats per critical path. |
| `full_stack_e2e` | bool | `false` | Runs `scripts/full-stack-e2e.sh`, which must exist. |
| `openapi_compatibility` | `"off"`, `"report"` or `"enforce"` | `"off"` | Compares the schema with the base revision. Needs strict and a backend. |
| `android_emulator_port` | even integer 5556 to 5810 | picked from the name | First emulator console port. Mobile only. |
| `android_emulators` | integer 1 to 4 | `2` | Emulators the gate boots. Mobile only. |
| `android_wifi` | bool | `false` | Gives each emulator its own Wi-Fi network. Mobile only. |
| `shared` | list of globs | `[]` | Extra paths whose change runs the full gate. |
| `light` | list of lane or step names | `[]` | Lanes or steps that take no core share. |
| `waits_for` | table of name to list | gate default | Read by `scripts/gate.py` only. Lanes or steps a step starts after. |
| `disk` | table | `floor_gb = 40` | Read by `scripts/gate.py` only. `floor_gb` and `[quality.disk.step_floor_gb]`. |

`baukit new` writes `android_emulator_port` from a hash of the product name into one of the 16
blocks of 8 ports from 5556 to 5683. Doctor rejects a range that runs past its block.

### `[quality.gate_budget_minutes]`

| Key | Type | Default |
| --- | --- | --- |
| `full` | integer above 0 | `10` |
| `affected` | integer above 0 | `5` |

The gate prints a warning over budget and never fails on it.

### `[quality.affected]`

Maps a relative glob to the lanes, steps or `playwright:@tag` tokens a change to it selects in
`--affected` runs. An empty list selects only the fast checks. Doctor rejects absolute globs,
globs with `..`, and names the product's gate does not list.

### `[quality.resources]`, `[quality.shared_resources]` and `[quality.creator_resources]`

Each maps a host resource name, such as `android` or `docker-network`, to the lanes or steps that
lock it while `BAUKIT_GATE_LOCK_DIR` is set. `resources` locks exclusively, `shared_resources`
lets holders share with each other, and `creator_resources` lets creators share with each other
but not with shared holders. A step named in more than one table takes the resource exclusively.

### `[quality.ci]`

Settings the generated `.github/workflows/ci.yml` quality job takes. `baukit new` reads them from
an existing manifest when it re-renders.

| Key | Type | Default |
| --- | --- | --- |
| `timeout_minutes` | integer 1 to 360 | `90` |
| `env` | table of `UPPER_SNAKE` name to string | `{}` |

### `[quality.cache]`

Settings for `scripts/gate_results.py`, the strict gate's result cache. `docs/quality-gate.md` in a
generated product describes when it skips a step.

| Key | Type | Default |
| --- | --- | --- |
| `never` | list of step names | `[]` |
| `max_age_days` | integer 0 to 65535 | `7` |

`never` lists steps that always run because they read data outside the tree. `baukit new` writes
`secret-scan` and `osv`, plus `cargo-deny` and `migrations` when the product has a backend. Doctor
rejects a name the product's gate does not list. A cached pass older than `max_age_days` runs again.

## `[gate]`

### `[gate.worker-caps]`

Caps one worker variable the strict gate exports. The gate uses the smaller of its share and the
cap. The keys are `BAUKIT_GATE_WORKERS`, `CARGO_BUILD_JOBS`, `NEXTEST_TEST_THREADS`,
`RUST_TEST_THREADS`, `VITEST_MAX_WORKERS`, `E2E_WORKERS`, `BAUKIT_QA_GRADLE_WORKERS` and
`BAUKIT_QA_NATIVE_JOBS`. Values are integers of at least 1. Reading the manifest fails on an
unknown key.

```toml
[gate.worker-caps]
E2E_WORKERS = 4
```

## `[auth]`

| Key | Type | Default | Read by |
| --- | --- | --- | --- |
| `login_theme` | string | `"baukit-accessible"` | `new` and doctor. The Keycloak login theme. |
| `mobile_client_id` | string | `"<app>-mobile"` | `new` and doctor. The mobile app's Keycloak client. |
| `mobile_redirect_path` | string | `"oauth"` | `new` and doctor. The path after `<scheme>://` in the mobile redirect URI. |

Each value holds letters, digits, `.`, `_` and `-` and starts with a letter or digit. The redirect
path may hold several such segments separated by `/`.

## `[ports]`

`port_offset` shifts the service host ports. `[ports]` pins single ports instead. Host ports apply
to local URLs and `make dev`. Container ports apply to Compose and deployment values.

```toml
[ports]
api = { host = 17001, container = 8080, service = "backend" }
ops = { host = 17002, container = 9090, service = "backend" }
postgres = { host = 17003, container = 5432 }
keycloak = { host = 17004, container = 8080 }
web = { host = 5673 }
web_preview = { host = 4773 }
metro = { host = 8093 }
web_extra = [8082, 19006]
```

| Name | Shape | Default host port | Notes |
| --- | --- | --- | --- |
| `postgres` | mapping | 5432 plus offset | Container port must stay 5432. |
| `api` | mapping | 8080 plus offset | |
| `ops` | mapping | 9090 plus offset | |
| `keycloak` | mapping | 8081 plus offset | Container port must stay 8080. |
| `redis` | mapping | 6379 plus offset | Container port must stay 6379. |
| `fake_provider` | mapping | 18081 plus offset | |
| `web` | `{ host }` | Vite's 5173 | Vite dev port, Keycloak web origins, backend `authorized_parties`, the PKCE helper default. |
| `web_preview` | `{ host }` | Vite's 4173 | Vite preview port and the hermetic Playwright `E2E_WEB_PORT`. |
| `metro` | `{ host }` | Expo's choice | Adds `--port` to mobile scripts that start Expo. |
| `local_callback` | `{ host }` | 18890 | Loopback PKCE callback of the Clerk and WorkOS `<app>-local` client. |
| `web_extra` | list of ports | none | More browser origins doctor accepts. Needs `web`. Templates do not render it. |
| `keycloak_extra` | list of ports | none | More loopback Keycloak ports whose issuers doctor accepts. Templates do not render it. |

A mapping is `{ host, container, service }`. `container` and `service` are optional, and `service`
renames the Compose service. The dev-server names take only `host`, and `port_offset` never
shifts them. Every host port must differ from every other effective host port. Lists must not be
empty, hold 0 or repeat a port. Declaring `metro = { host = 8081 }` collides with the default
Keycloak port.

Doctor checks only declared dev ports, and skips files the product removed:

- `web`: `web/vite.config.ts` sets `server.port`, the stack Playwright config and
  `scripts/pkce-login.py` use the same port, and Keycloak origins and backend
  `authorized_parties` accept only loopback origins on `web`, `web_preview`, `metro` and
  `web_extra` ports.
- `web_preview`: `web/e2e/playwright.config.ts` uses it as the `E2E_WEB_PORT` default.
- `metro`: every `expo start` or `expo run:*` script in `mobile/package.json` passes `--port`,
  and `adb reverse` lines and `RCT_METRO_PORT=` settings that name Metro agree.

The mobile QA scripts default PostgreSQL, Redis, the API, the ops listener and Keycloak to the
effective host port plus 10000, so offset 0 gives 15432, 16379, 18080, 19090 and 18081.
`BAUKIT_QA_*_PORT` variables and `mobile/scripts/qa/local.sh` still win.

To apply a port change to the files the template rendered, copy the updated manifest into an
empty directory, render there and copy the affected files back:

```sh
mkdir -p .port-render
cp baukit.toml .port-render/
baukit new my-product --backend --web --mobile --auth oidc \
  --into-existing --dir .port-render --skip-lockfiles
```

Use the product's own capability flags, and `--baukit-path` if it has a path dependency.

## `[platform]`

Present only with `capabilities.platform = true`, and doctor fails when the two disagree.

| Key | Type | Read by |
| --- | --- | --- |
| `migration` | string | `new` and doctor. The forward SQL migration that creates the platform tables. |

Hub URLs, secrets and tokens are runtime config, not manifest keys. See
[suites and the platform](suite-and-platform.md).

## `[postgres]`

| Key | Type | Default | Read by |
| --- | --- | --- | --- |
| `owner_create_role` | bool | `false` | `new` and doctor. Gives the migration owner `CREATEROLE`. |

Role names are global to a Postgres cluster, so doctor allows the flag only when every GitOps
overlay uses dedicated Postgres.

## `[gitops]`

Written for every backend product. Read by `baukit new` and `baukit generate gitops` when they
render `deploy/gitops/`.

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `domain` | lowercase DNS name | `"lab.example.com"` | Testing serves `<app>-testing.<domain>`, production `<app>.<domain>`. |
| `registry` | lowercase registry path | `"ghcr.io/example"` | Images are `<registry>/<app>-<process>`. |
| `lab_registry` | lowercase registry path | unset | When set, overlays pull from it and the release copies each signed image there. |
| `identity_url` | https URL | `https://id.<domain>` | Keycloak base URL. |
| `realm` | lowercase name | the app name | Keycloak realm. |

## `[doctor]`

Overrides for products whose layout moved away from the template. Every path is relative to the
product root and has no `..`.

| Key | Default |
| --- | --- |
| `backend_manifest` | `backend/Cargo.toml` |
| `backend_dockerfile` | `Dockerfile` next to `backend_manifest` |
| `backend_dockerignore` | `.dockerignore` next to `backend_manifest` |
| `migrations` | `backend/migrations` |
| `keycloak_realm`, `keycloak_policy`, `keycloak_reconcile` | found by content |
| `keycloak_policy_tool`, `keycloak_reconcile_tool` | found by content |
| `waive` | `[]` |

`waive` is a list of `{ path, reason }` for expected files the product covers another way. Doctor
prints the reason as a note. It fails on an empty reason, a duplicate path, a path that exists, or
a path that is not on the product's expected-file list.

```toml
[doctor]
waive = [{ path = "web/e2e/tests/qa-console.spec.ts", reason = "Every browser test uses the console fixture in web/e2e/tests/qa.ts" }]
```

### `[doctor.sources]`

Each key points one wiring check at one file. The file must still contain the wiring.

| Key | Doctor looks for |
| --- | --- |
| `backend_limits` | A shared limits check such as `check_measurement` |
| `worker_entry` | A `main` using `ProcessKind::Worker` |
| `worker_tests` | `WorkerRunner` or a PostgreSQL test database fixture |
| `auth_tests` | `check_auth_router_conformance` or `MockOidcServer` |
| `pkce_login` | `code_challenge` and `S256` in the optional login helper |
| `mobile_sign_in` | `signIn`, `signInWithOidc` or `login` in an app route |
| `mobile_auth` | `createExpoOidcClient`, `createNativeOidcClient` or `NativeOidcClient` |
| `mobile_auth_tests` | `signIn` or `signInWithOidc` in a test |
| `mobile_local_data` | `ScopedPersistenceRegistryStore` |
| `mobile_persistence` | `ScopedPersistenceLifecycle` |
| `keycloak_policy_tests` | unittest coverage of `validate_realm` |
| `keycloak_reconcile_tests` | unittest coverage of `load_reconcile_config`, `validate_inputs` or `RealmReconciler` |
| `mcp_router`, `mcp_config`, `mcp_tools`, `mcp_drift` | The remote MCP router, config, tool registration and drift test |

## `[backend.image]`

`baukit generate dockerfile` renders `backend/Dockerfile` from this table, and doctor fails when
the committed file differs. Without the table the Dockerfile builds `api` and `migrate` from
`<app>-bin`, plus `worker` when `capabilities.worker` is true. Each binary gets its own distroless
stage with entrypoint `/app/<binary>`, so `docker build --target api` picks one. The `migrate`
stage also holds the migrations and the bin crate's `Cargo.toml` at their compile-time paths.

| Key | Default | Meaning |
| --- | --- | --- |
| `binaries` | from capabilities | Replaces the default binary list. |
| `bin_crate` | `"<app>-bin"` | The Cargo package that builds the binaries. |
| `backend_context` | `"."` | Default of the `BACKEND_CONTEXT` build arg. `"backend"` for a repository-root context. |
| `cargo_build_jobs` | unset | Default of `ARG CARGO_BUILD_JOBS`. |
| `apt_packages` | `[]` | Builder packages, installed with `--no-install-recommends`. |
| `build_inputs` | `[]` | `{ source, destination }` copies from the build context into the builder. |
| `pre_build` | `[]` | `{ command, outputs }`. `command` is an argv list run before Cargo in the same cache mount. |
| `runtime_files` | `[]` | `{ stage, source, destination }` copies from the builder into a runtime stage. |
| `runtime_binaries` | `[]` | `{ stage, binary }` companion binaries copied to `/app/<binary>`. |
| `runtime_packages` | `[]` | `{ stage, packages }`. The stage uses Debian slim with `ca-certificates` and these packages. |
| `writable_directories` | `[]` | `{ stage, path }` directories owned by `65532:65532`. |
| `downloads` | `[]` | `{ stage, url, archive_sha256, binary, binary_sha256, destination }` tar.gz downloads. |
| `variants` | `[]` | `{ name, features, runtime_packages }` extra builds with Cargo features. |

```toml
[backend.image]
backend_context = "backend"
cargo_build_jobs = 6
build_inputs = [{ source = "fixtures", destination = "/fixtures/" }]

[[backend.image.runtime_packages]]
stage = "worker"
packages = ["git"]

[[backend.image.variants]]
name = "smoke"
features = ["stack-fixtures"]
runtime_packages = [{ stage = "api", packages = ["socat"] }]
```

Validation, from `cli/src/dockerfile.rs`:

- Unknown keys fail. Paths use letters, digits, `/`, `.`, `_` and `-`, with no `..`.
  Build-context sources are relative. Runtime sources outside `/workspace` must be a
  `pre_build` output or below one.
- Every `stage` names a declared binary. `runtime_packages` stages do not repeat, and package names
  start with a letter or digit and hold letters, digits, `.`, `+` or `-`.
- Download URLs are https, and both checksums are 64 hex digits. The Dockerfile uses
  `ADD --checksum`, checks the extracted binary and installs it as root with mode `0555`.
- Variant names match `[a-z][a-z0-9-]*` and do not repeat. Each variant has at least one feature.
  `builder-<name>` builds every binary with those features into its own
  `<app>-target-<name>` cache, and each binary gets a `<binary>-<name>` stage after the base stages.
- `pre_build` arguments are literals. Use `["sh", "-c", "..."]` for shell syntax. Outputs needed
  at runtime must live outside `target/`, which is a cache mount.

The Rust, distroless and Debian slim bases are digest-pinned by baukit, not by the manifest. The
`RUST_IMAGE`, `RUNTIME_IMAGE` and `PACKAGES_RUNTIME_IMAGE` build args override them. The
Dockerfile always sets `SQLX_OFFLINE=true`, so builds use the committed `.sqlx` metadata. It also
takes `BACKEND_CONTEXT`, `BAUKIT_CONTEXT`, `BAUKIT_DESTINATION`, `LIMITS_FILE`, `GIT_COMMIT` and
`CARGO_TARGET_CACHE_PREFIX`. `scripts/docker.sh` sets that last one to `<app>-<hash>`, where the
hash is the first 12 hex digits of the SHA-256 of the checkout path, so two checkouts never share
a Cargo target cache:

```sh
sh scripts/docker.sh build --target api -f backend/Dockerfile \
  --build-arg BACKEND_CONTEXT=backend .
```

Bind addresses, ports and log settings belong in Helm or Compose. The image has no `EXPOSE`.

## Reference

- `cli/src/manifest.rs`, `cli/src/ports.rs`, `cli/src/dockerfile.rs`, `cli/src/gate_settings.rs`,
  `cli/src/gate_worker_caps.rs`, `cli/src/doctor_layout.rs`
- [`cli/README.md`](../../cli/README.md)
- [The baukit CLI](cli.md)
- [Quality gates](quality-gates.md)
- [Deploy a product](deploying.md)
- [ADR 0010: scaffolding lifecycle](../adr/0010-scaffolding-lifecycle.md)
- [ADR 0011: separate API and ops listeners](../adr/0011-separate-api-and-ops-listeners.md)
