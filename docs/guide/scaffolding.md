# Scaffold a product

`baukit new` renders a product from the templates in `templates/`. This page explains each
choice the command offers, what it puts on disk, and how to add files to a product that already
exists. At the end you know which flags to pass and what to commit.

## Choose the apps

At least one of these is required:

| Flag | Renders |
| --- | --- |
| `--backend` | A Rust workspace under `backend/`, migrations, `compose.yaml`, OpenAPI drift tests, Helm values and the production Dockerfile. |
| `--web` | A Vite React app under `web/` with Playwright tests. |
| `--mobile` | An Expo React Native app under `mobile/` with Jest and the Android QA scripts. |

Then add capabilities:

| Flag | Needs | Adds |
| --- | --- | --- |
| `--worker` | `--backend` | A `worker` binary and crate that drain the durable job outbox. |
| `--auth oidc\|clerk\|workos` | | Backend token checks and sign-in in each frontend. `oidc` also bundles a Keycloak realm. |
| `--mcp` | `--backend`, `--auth` | A remote MCP server crate and its tool drift test. |
| `--platform` | `--backend` | The platform receiver and entitlement cache, plus a `[platform]` table. |
| `--pwa` | `--web` or `--mobile` | A service worker build. |

`--quality strict` swaps the per-concern CI jobs for one gate script. See
[run the quality gate](quality-gates.md).

The source of truth for the flags is `cli/src/main.rs`. The [CLI reference](cli.md#baukit-new)
lists every one.

## Name the product

The name is lowercase kebab-case, 1 to 41 characters, starting with a letter. It becomes the
crate prefix (`notes-api`, `notes-bin`), the config env prefix (`NOTES__`), image names, the
Keycloak realm and the analytics context. Pick it once. Renaming later means renaming all of
those by hand.

## What you get

`baukit new docscheck --backend --worker --web --mobile --auth oidc --mcp` renders, among others:

```text
baukit.toml               the manifest
.baukit/template-files.toml  hashes of template-owned files
CLAUDE.md, AGENTS.md      agent rules, AGENTS.md is a symlink
Makefile, compose.yaml, mise.toml, limits.json
backend/                  Cargo workspace, migrations, fuzz, Dockerfile, openapi.json
web/, mobile/             the frontends
deploy/                   Helm values, GitOps overlays, observability
docs/                     product docs, ADRs, runbooks
scripts/                  setup, preflight, release, lockfile and Keycloak scripts
.github/workflows/        ci.yml, native.yml, release.yml, promote.yml
```

The backend workspace splits by layer: `<app>-domain`, `<app>-ports`, `<app>-services`,
`<app>-api`, `<app>-postgres`, `<app>-bin`, plus `<app>-worker` and `<app>-mcp` when enabled.
[The backend guide](backend.md) walks through it.

Commit everything, including `baukit.toml`, `.baukit/template-files.toml` and the lockfiles.

## Where baukit comes from

By default the product depends on released crates and npm packages, and `baukit.toml` records
`source = "registry"` with the version. To develop against a local baukit checkout instead:

```sh
baukit new notes --backend --web --baukit-path ../baukit/rust
```

That writes `source = "path"`. Doctor then checks that the path holds the baukit crates.

## Ports

Every product starts on the same default ports. To run two products side by side, give the
second an offset:

```sh
baukit new other --backend --web --port-offset 100
```

The offset shifts the service host ports and lands in `baukit.toml`. Dev-server ports stay put.
The [`[ports]` reference](manifest.md#ports) covers pinning single ports.

## Skip the network

`--skip-lockfiles` renders without resolving `Cargo.lock` and `pnpm-lock.yaml`. Run
`make lockfiles` later, before the first build.

## Add files to an existing product

Re-rendering adds files a newer template or a new flag brings, and never overwrites your edits.

```sh
baukit new notes --backend --web --auth oidc --into-existing --dir .
```

`--into-existing` renders straight into `--dir`, the repository root. `--force` does the same into
`DIR/NAME` when that directory is not empty. In both modes:

- Missing files are written.
- Files identical to the render are left alone.
- Files that differ are kept and listed in `baukit-conflicts.txt`, and the command exits 1. Merge
  each one by hand, then delete the list.
- The existing `baukit.toml` stays as it is, and its ports, auth and gitops settings drive the
  render.
- `.baukit/template-files.toml` is rewritten.

Pass the capability flags the product already has, plus any new one. Then run `baukit doctor`
and `baukit check template-files --update`. For GitOps alone, `baukit generate gitops` adds only
those files.

[ADR 0010](../adr/0010-scaffolding-lifecycle.md) explains why the CLI adds files instead of
patching them.

## Reference

- `cli/src/main.rs`, `cli/src/generate/`
- [`cli/README.md`](../../cli/README.md)
- [The baukit CLI](cli.md)
- [`baukit.toml` reference](manifest.md)
- [ADR 0009: hexagonal generated backend](../adr/0009-hexagonal-generated-backend.md)
- [ADR 0010: scaffolding lifecycle](../adr/0010-scaffolding-lifecycle.md)
