# Use the baukit CLI

This page is the reference for every `baukit` subcommand and flag. Read it when you scaffold a
product, check one, regenerate a derived file, or need to know what a `baukit check` rule
enforces. The clap definitions in `cli/src/main.rs` are the source, and
`cli/tests/docs_reference.rs` fails when a subcommand, flag or rule is missing here.

## Install

The CLI is not on crates.io. Install it from the release tag that matches the product's
`template_version`, so its embedded templates and dependency versions agree:

```sh
cargo install --git https://github.com/PatrickKoss/baukit --tag vX.Y.Z --locked baukit-cli
baukit --version
```

`baukit --version` prints the template version compiled into the binary. Inside the baukit
checkout you can run the CLI without installing it:

```sh
cargo run --manifest-path cli/Cargo.toml --bin baukit -- doctor
```

## Subcommands

| Command | Does |
| --- | --- |
| `baukit new NAME` | Renders a new product from the templates. |
| `baukit doctor` | Validates the product in the current directory against its `baukit.toml`. |
| `baukit generate dockerfile` | Renders `backend/Dockerfile` from `[backend.image]`. |
| `baukit generate openapi-client` | Writes TypeScript declarations for every `openapi.consumers` entry. |
| `baukit generate gitops` | Adds the GitOps overlays and release workflows a product is missing. |
| `baukit generate release-age-excludes` | Prints the pnpm `minimumReleaseAgeExclude` blocks for a fresh `@baukit/*` pin. |
| `baukit check [RULE...]` | Runs the shared code rules. |

`new`, `doctor` and `generate` exit 0 on success and 1 on any error. `check` has its own exit
codes, listed below.

## `baukit new`

```sh
baukit new NAME [--backend] [--worker] [--mobile] [--web] [--pwa] [--mcp] [--platform]
  [--auth oidc|clerk|workos] [--dir DIR] [--force] [--into-existing] [--skip-lockfiles]
  [--baukit-path PATH] [--port-offset N] [--quality standard|strict]
```

`NAME` is lowercase kebab-case: 1 to 41 ASCII letters, digits and single hyphens, starting with
a letter. The limit keeps generated Kubernetes Service names under 63 characters.

| Flag | Default | Effect |
| --- | --- | --- |
| `--backend` | off | Rust backend workspace under `backend/`. |
| `--worker` | off | Durable PostgreSQL worker in the backend workspace. Needs `--backend`. |
| `--mobile` | off | Expo React Native app under `mobile/`. |
| `--web` | off | Vite React app under `web/`. |
| `--pwa` | off | Service worker build for the web app, or for the Expo web export without `--web`. Needs `--mobile` or `--web`. |
| `--mcp` | off | Remote MCP server crate. Needs `--backend` and `--auth`. |
| `--platform` | off | Platform receiver and entitlement cache in the backend. Needs `--backend`. |
| `--auth` | none | `oidc`, `clerk` or `workos`. |
| `--dir` | `.` | Parent directory. The product lands in `DIR/NAME`. |
| `--force` | off | Writes into a non-empty destination. Adds missing files and reports differing ones as conflicts. |
| `--into-existing` | off | Renders straight into `--dir` instead of `DIR/NAME`, for an existing repository root. |
| `--skip-lockfiles` | off | Skips the network-backed Cargo and pnpm lockfile resolution after rendering. |
| `--baukit-path` | unset | Depends on crates and packages in a local checkout's `rust/` directory instead of the registries. |
| `--port-offset` | `0` | Adds N to the generated local service ports and records it in `baukit.toml`. |
| `--quality` | `standard` | Selects the CI and local gate profile, `standard` or `strict`. |

At least one of `--backend`, `--mobile` and `--web` is required. With `--force` or
`--into-existing`, the CLI never overwrites a file whose content differs. It lists those files in
`baukit-conflicts.txt`, or `baukit-conflicts.1.txt` and so on when that name is taken, and exits 1.
When the destination already holds `baukit.toml`, the render uses that manifest's ports, auth,
gitops and CI settings and keeps the manifest unchanged.

[Scaffold a product](scaffolding.md) explains what each flag renders and how to re-render an
existing product.

## `baukit doctor`

`baukit doctor` takes no flags. Run it from the product root. It reads `baukit.toml`, runs every
check, prints `ok: ...` for each pass and `note: ...` for each warning, and ends with
`doctor: product is healthy`. On a failure it prints `doctor found N problem(s):` with one line
per problem and exits 1.

The checks run in this order, from `doctor_with_host` in `cli/src/doctor.rs`:

1. Declared paths. Every path under `[doctor]` is relative and has no `..`.
2. Expected files. Each template file the product's flavors, capabilities and profile expect
   exists, unless a `[doctor] waive` entry covers it.
3. Quality settings. `quality.full_stack_e2e` has its script, `backend_coverage_lines` is 1 to 100,
   and the `[auth]` names are valid.
4. Gate steps. Gate settings are valid and, in a strict product, `scripts/quality-gate.sh
   --list-steps` lists every template step for the declared capabilities.
5. Emulator ports. Prints the Android emulator ports and block the settings take.
6. OpenAPI compatibility. `webkit_repeats` is above 0, and `openapi_compatibility` other than
   `off` has the strict profile and a backend.
7. Critical paths. Each `quality.critical_paths` entry is an existing `e2e/tests/*.spec.ts` file
   under `web/`.
8. Capability combinations. PWA has web or mobile, platform and MCP have a backend, MCP has auth,
   and `capabilities.platform` matches the `[platform]` table.
9. Versions. `schema_version` and `template_version` match the CLI.
10. Auth provider. The provider in `capabilities.auth` matches backend config and frontend SDK
    wiring.
11. Identity. Analytics contexts, Expo config and backend config namespaces agree on the product
    name.
12. Ports. Compose mappings, defaults and loopback URLs match `[ports]` and `port_offset`.
13. Backend. OpenAPI files, backend wiring, migrations, the `job_outbox` migration for a worker,
    the Keycloak realm tools, platform wiring and remote MCP wiring.
14. PostgreSQL. `postgres.owner_create_role` has a backend and dedicated Postgres in every
    GitOps overlay.
15. GitOps. `deploy/gitops/base/chart-source.yaml` pins the chart at the template version.
16. Cargo workspaces. `cargo metadata --no-deps` parses each backend workspace.
17. Mobile. Expo dependencies, router configuration, the QA build flag and auth wiring. Prints
    notes for hardcoded suite schemes and pinned QA ports.
18. Web. Vite, TanStack Query, analytics and auth dependencies.
19. PWA. The service worker build of the app that serves it.
20. Baukit dependency. A path dependency resolves, a git tag is reachable over SSH, or every
    registry pin equals `[dependencies.baukit] version`.
21. Docker. Docker and BuildKit are available when the product builds images, with SSH forwarding
    for a git dependency.

### Waivers and source overrides

Doctor looks for wiring by content, so files can move. Point a check at a specific file with
`[doctor.sources]`, or waive an expected file the product covers another way with
`[doctor] waive`. A waived file prints `note: waived missing expected ... file`. Both are
described in the [`[doctor]` section of the manifest reference](manifest.md#doctor).

## `baukit generate`

All four run from the product root.

### `baukit generate dockerfile`

Renders `backend/Dockerfile` from `[backend.image]` in `baukit.toml`. With `--check` it leaves the
file alone and fails when the committed file is missing or differs from the render. Doctor runs
the same comparison. The output path is `doctor.backend_dockerfile`, or the directory of
`doctor.backend_manifest`. See [`[backend.image]`](manifest.md#backendimage).

### `baukit generate openapi-client`

Runs `openapi-typescript@7.13.0` on `openapi.schema` for each path in `openapi.consumers`. It needs
`corepack`, `pnpm` or `npx` on `PATH`, tried in that order. It fails when the product has no
backend or lists no consumers. `scripts/openapi-client.sh` in a generated product does the same
from the shell.

### `baukit generate gitops`

Adds the files a product without GitOps lacks: `deploy/gitops/**`,
`deploy/cosign-signing-config.json`, `.github/workflows/release.yml`,
`.github/workflows/promote.yml`, and the release, promote, signing and pin scripts under
`scripts/`. It renders them from `baukit.toml`, writes only missing files, and prints
`added PATH` or `kept PATH (differs from the template)`. It fails without a backend. Run
`baukit check template-files --update` afterwards and follow `deploy/gitops/README.md`.
[Deploy a product](deploying.md) covers the rest.

### `baukit generate release-age-excludes`

pnpm refuses a package for a day after its publish when `minimumReleaseAge` is set. This command
walks the product for pnpm apps, a directory with `pnpm-workspace.yaml` or `pnpm-lock.yaml`, and
prints for each one the `@baukit/*` packages it pins plus every `@baukit/*` package those pull in,
at the pinned version:

```text
# web/pnpm-workspace.yaml, until @baukit/*@0.31.0 is a day old
minimumReleaseAgeExclude:
  - '@baukit/api-runtime@0.31.0'
```

Paste each block into the named file and remove it once the release is a day old. It fails when
one app pins two different `@baukit/*` versions. It skips `fixtures`, `node_modules`, `target` and
`vendor` directories.

## `baukit check`

```sh
baukit check [RULE...] [--root DIR] [--verbose] [--fix] [--update]
```

With no rule it runs all of them, in the order below. It prints one line per finding,
`rule: path[:line]: message`, then a summary line per rule. Lines marked `note:` never fail.

| Exit code | Meaning |
| --- | --- |
| 0 | No findings. |
| 1 | At least one finding. |
| 2 | Usage or config error, such as an unknown rule, an invalid `baukit-check.toml`, or a missing tool. |

| Flag | Effect |
| --- | --- |
| `--root` | Directory to check. Default `.`. |
| `--verbose` | Also prints hint-level `patterns` matches. |
| `--fix` | `agent-docs` writes the managed `CLAUDE.md` block and the `AGENTS.md` symlink. |
| `--update` | `suppressions` lowers the maxima in `baukit-check.toml` to the current counts. `template-files` rewrites the recorded hashes. |

### Rules

| Rule | Fails when |
| --- | --- |
| `lints` | A Cargo workspace lacks the `lint/rust/lints.toml` baseline or the `clippy.toml` from `lint/rust/`, a member skips `[lints] workspace = true`, or an eslint config does not import `@baukit/eslint-config`. |
| `file-size` | A Rust file passes 1000 lines or a TS or TSX file passes 500. Tests and generated files are exempt. |
| `suppressions` | A suppression has no reason, `@ts-ignore` appears, a count passes its `[suppressions.max]`, or a bulk suppression exists. Needs `ruff` for `# noqa` in Python files. |
| `patterns` | An error-level ast-grep rule from `lint/ast-grep/` matches. The CLI embeds ast-grep 0.51.1. |
| `agent-docs` | `CLAUDE.md` lacks exactly one managed block equal to `lint/agent-rules.md`, or `AGENTS.md` is not a symlink to `CLAUDE.md`. |
| `observability` | A dashboard or rule from `[observability]` in `baukit-check.toml` names an unknown metric or breaks a naming rule. Without that table it prints a note and passes. |
| `template-files` | A file listed in `.baukit/template-files.toml` no longer has its recorded SHA-256. Without the manifest it prints a note and passes. |
| `fuzz-lockfile` | A strict product with `backend/fuzz/Cargo.toml` tracks `backend/fuzz/Cargo.lock` in git. |
| `security` | Production wiring skips a control. See below. |

`security` reads non-test Rust, YAML and TOML files. It reports:

- `axum::serve` without `into_make_service_with_connect_info`, because the IP rate limiter then
  cannot see the client address.
- `.merge(x)` in a crate whose directory ends in `-bin` where `x` does not start with
  `rate_limited(`, `protect_routes(` or `baukit_ratelimit::layers(`.
- `opsService.type` in YAML set to anything but `ClusterIP`.
- A CORS origin that is `null`. A CORS origin is a string under a key such as `allowedOrigins`,
  `cors_allowed_origins` or `publicApiOrigin`. A bare `origin` key does not count, and YAML files
  with a top-level `openapi`, `swagger` or `$schema` key are not checked for origins.
- A CORS origin that does not start with `https://`, unless the file path contains `local`.
  `http://localhost`, `http://127.0.0.1` and `http://[::1]`, with any port, also pass in compose
  files, CI workflows and `e2e` directories.

The rule splits JSON arrays and comma-separated lists and judges each origin, and it resolves
`${VAR:-default}` to its default. Test code is skipped: `tests.rs`, `*_tests.rs`, `tests`,
`benches`, `examples`, `e2e`, `baukit-test` and `*-test-support` directories, and everything after
the first `#[cfg(test)]`.

`lint/README.md` documents `baukit-check.toml`, every suppression key and the bulk-suppression
forms. The [quality gates guide](quality-gates.md) shows where the gate runs these rules.

### Fixes

- `baukit check agent-docs --fix` writes or replaces the managed block and creates the `AGENTS.md`
  symlink. It refuses when `AGENTS.md` is a regular file that differs from `CLAUDE.md`.
- `baukit check suppressions --update` lowers `[suppressions.max]`. It never raises a value.
- `baukit check template-files --update` rewrites the hashes after a deliberate upgrade, drops
  rows whose file is gone, and adds rows for template-owned files the product holds unchanged.
  A diverged template-owned file prints `template-owned but diverged, not tracked`.

## Reference

- [`cli/README.md`](../../cli/README.md)
- [`lint/README.md`](../../lint/README.md)
- [`baukit.toml` reference](manifest.md)
- [Scaffold a product](scaffolding.md)
- [ADR 0010: scaffolding lifecycle](../adr/0010-scaffolding-lifecycle.md)
- [ADR 0013: code rules enforced by baukit check](../adr/0013-code-rules-enforced-by-baukit-check.md)
- `cli/src/main.rs`, `cli/src/check/mod.rs`, `cli/src/doctor.rs`
