# Get started with baukit

This page takes you from a clean machine to a generated product whose checks pass. At the end
you have the `baukit` CLI installed, a product with a backend and web app, its local stack
running, and `baukit doctor` reporting it healthy.

## What you need

- Rust through rustup. The generated backend sets `rust-version = "1.95"` and the product's
  `mise.toml` pins the toolchain it gates with.
- Node with corepack. Generated apps pin pnpm in `packageManager`, and every command uses
  `corepack pnpm`.
- Docker with BuildKit. Compose runs PostgreSQL, Keycloak and Redis, and the backend tests that
  need a database are `#[ignore]` tests the gate runs with `--include-ignored`.
- Python 3 for the generated scripts.
- [mise](https://mise.jdx.dev) if you pick the strict profile. It installs the linters and
  scanners the gate calls.

## Install the CLI

The CLI is not on crates.io. Install it from a release tag:

```sh
cargo install --git https://github.com/PatrickKoss/baukit --tag vX.Y.Z --locked baukit-cli
baukit --version
```

Use the newest tag. The version you install is the version the product starts on, because the CLI
embeds its templates.

## Generate a product

```sh
baukit new notes --backend --web --auth oidc
cd notes
```

`notes` lands in the current directory. The command renders the files, resolves the Cargo and
pnpm lockfiles over the network, and writes `baukit.toml`. Add `--mobile`, `--worker` or
`--quality strict` as you need them. [Scaffold a product](scaffolding.md) explains every
choice.

## Run it

From the product root:

```sh
make setup        # writes .env files from the .env.example files
make db-up        # starts PostgreSQL in Compose
export NOTES__DATABASE__URL=postgres://app:app@localhost/notes
make migrate      # applies backend/migrations
make dev          # reconciles the Keycloak realm and starts Redis
make run          # starts the API on 8080 and the ops listener on 9090
```

Without `NOTES__DATABASE__URL` the API uses an in-memory repository. It never runs migrations
on startup. Source: the generated `Makefile` and `README.md`. Check the process from another shell:

```sh
curl -i http://localhost:9090/healthz
curl -i http://localhost:9090/readyz
```

The web app runs from `web/` with `corepack pnpm install && corepack pnpm dev`. When another
product holds these ports, set `port_offset` or `[ports]` in `baukit.toml`, as the
[manifest reference](manifest.md#ports) describes.

## Check it

```sh
baukit doctor
baukit check
make check
```

`baukit doctor` validates the product against `baukit.toml` and ends with
`doctor: product is healthy`. `baukit check` runs the shared code rules. `make check` runs
format, clippy with `-D warnings`, the backend tests including the ignored ones, the script tests,
and the web checks. In a strict product, run `sh scripts/quality-gate.sh` instead. The
[quality gate guide](quality-gates.md) covers both.

## Next steps

- Add an endpoint: [the backend guide](backend.md).
- Learn the manifest: [`baukit.toml` reference](manifest.md).
- Give coding agents the baukit skills: [agent skills](agent-skills.md).
- Move to a new release: [upgrade a product](upgrading.md).

## Reference

- [`cli/README.md`](../../cli/README.md)
- [The baukit CLI](cli.md)
- [Scaffold a product](scaffolding.md)
- [ADR 0007: toolkit of opt-in units](../adr/0007-toolkit-of-opt-in-units.md)
- [ADR 0010: scaffolding lifecycle](../adr/0010-scaffolding-lifecycle.md)
