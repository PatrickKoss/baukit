# baukit

[![CI](https://github.com/PatrickKoss/baukit/actions/workflows/ci.yml/badge.svg)](https://github.com/PatrickKoss/baukit/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/baukit-runtime.svg?label=crates.io)](https://crates.io/crates/baukit-runtime)
[![npm](https://img.shields.io/npm/v/@baukit/api-runtime.svg?label=npm)](https://www.npmjs.com/package/@baukit/api-runtime)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

The boring 80% of a Rust backend and its TypeScript clients, already built and already tested.

*Baukit* is German for a construction kit, and that is the whole idea. Rust crates and npm packages, each small enough to adopt on its own, plus a CLI that wires them into a working product when you want the whole thing.

> [!WARNING]
> **Under heavy development. Expect breaking changes.**
> Every component is on one `0.x` train and minor bumps break API before 1.0. Only the latest minor is supported, and there are no backports. **Pull requests are not accepted yet.** Issues and discussions are welcome.

## The problem

Every new service starts with the same three weeks. Graceful shutdown that actually drains. Health endpoints that a load balancer can distinguish from readiness. Config from files and environment with real validation. An error envelope your frontend can branch on. Trace IDs that survive the hop from browser to backend. Prometheus metrics named consistently enough that one dashboard works for every service.

None of it is hard. All of it is fiddly, all of it gets done slightly differently each time, and all of it is what breaks at 3am.

Baukit is that work, extracted from real products, with the tests attached.

## Quickstart

You need Docker, because `make test` runs the Docker-backed integration tests. Scaffold a product:

```bash
cargo install --git https://github.com/PatrickKoss/baukit --tag v0.32.0 --locked baukit-cli
baukit new orders --backend --web
cd orders && make check
```

You get a hexagonal Rust backend (`orders-domain`, `-ports`, `-services`, `-api`, `-postgres`, `-bin`), a Vite React app with a typed client generated from the backend's OpenAPI schema, Docker builds, a Helm values file, CI, and conformance tests that fail if the health and metrics contracts drift. Add `--quality strict` for the stricter gate with coverage, migration, OpenAPI, browser, image and native checks. [Getting started](docs/guide/getting-started.md) has the prerequisites per flavor and the full walkthrough.

Generated products pull baukit from crates.io and npm by pinned version, so nothing here needs repository access or an SSH key. Point the CLI at a local checkout with `--baukit-path rust` when you want path and `file:` dependencies instead.

Or add a crate to a service you already have:

```toml
[dependencies]
baukit-http = "0.32"
baukit-ops = "0.31"
```

## What it looks like running

The [`examples/minimal-api`](examples/minimal-api) notes service fits in one short file. Start it:

```console
$ cargo run --manifest-path examples/minimal-api/Cargo.toml
INFO minimal_api: service started api_address="0.0.0.0:8080" operations_address="0.0.0.0:9090" service="minimal-api-api"
```

Two listeners, and that split matters. Your public API is on 8080. Everything an operator needs is on 9090, which you never expose to the internet.

A request comes back with identity and trace context already attached, and you wrote no middleware to do it.

```console
$ curl -i -X POST localhost:8080/notes -d '{"title":"first","body":"hello"}' -H 'content-type: application/json'
HTTP/1.1 201 Created
x-request-id: 11d29641-2e48-471d-83fc-c04de54377ab
traceparent: 00-e82b00b429435b4cda9686a7f66779de-56ded6437f5d3bad-01
access-control-expose-headers: x-request-id,traceparent,tracestate

{"id":1,"title":"first","body":"hello"}
```

Failures use one envelope across every endpoint, with the request ID inside the body so a user can paste it into a bug report and you can find the trace:

```console
$ curl -X POST localhost:8080/notes -d '{"title":"","body":"x"}' -H 'content-type: application/json'
{"error":{"code":"validation_failed","message":"The request is invalid",
          "requestId":"0f9bead5-9dd2-4f45-b876-0e12e9d663a3",
          "details":{"title":"must not be empty"}}}
```

`@baukit/api-runtime` parses that shape on the client, so a validation failure arrives in your React code as a typed error with `details.title` rather than a string you have to guess at.

On the operations port, readiness is a real answer instead of a 200:

```console
$ curl localhost:9090/readyz
{"status":"ready","accepting_traffic":true,
 "checks":[{"name":"accepting_traffic","status":"pass","duration_ms":0},
           {"name":"state","status":"pass","duration_ms":0}],
 "diagnostics":[]}

$ curl localhost:9090/buildinfo
{"service_name":"minimal-api-api","version":"0.1.0","commit":"unknown","rust_version":"1.100.0"}
```

`/healthz` says the process is alive, `/readyz` says it should receive traffic, and they are genuinely different during a deploy. When shutdown starts, the traffic gate flips `/readyz` to failing while in-flight requests finish draining, which is the difference between a rolling deploy and a handful of dropped connections.

Metrics come out already named the way the shipped Grafana dashboards expect: `http_requests_total`, `http_request_duration_seconds`, `http_requests_in_flight`, `build_info`. A linter in CI keeps them that way.

## Capabilities

Nothing here depends on the CLI, and no crate drags in the rest. Take the error envelope and skip the job queue. The [guide index](docs/guide/README.md) has one line per crate and package.

| Capability | Crates | Packages | Guide |
|---|---|---|---|
| A service that starts, drains and reports health | [`baukit-runtime`](rust/crates/baukit-runtime), [`baukit-config`](rust/crates/baukit-config), [`baukit-http`](rust/crates/baukit-http), [`baukit-ops`](rust/crates/baukit-ops), [`baukit-openapi`](rust/crates/baukit-openapi), [`baukit-core`](rust/crates/baukit-core) | [`@baukit/api-runtime`](typescript/packages/api-runtime) | [Backend](docs/guide/backend.md) |
| Sign-in and token verification | [`baukit-auth`](rust/crates/baukit-auth) | [`@baukit/auth-web`](typescript/packages/auth-web), [`@baukit/auth-native`](typescript/packages/auth-native), [`@baukit/auth-node`](typescript/packages/auth-node) | [Authentication](docs/guide/auth.md) |
| Offline data and sync | [`baukit-sync`](rust/crates/baukit-sync) | [`@baukit/sync-client`](typescript/packages/sync-client), [`@baukit/data-contracts`](typescript/packages/data-contracts), [`@baukit/data-contracts-dexie`](typescript/packages/data-contracts-dexie), [`@baukit/data-contracts-expo-sqlite`](typescript/packages/data-contracts-expo-sqlite) | [Data and sync](docs/guide/data-and-sync.md) |
| Background jobs | [`baukit-jobs`](rust/crates/baukit-jobs) | none | [Jobs](docs/guide/jobs.md) |
| Remote MCP for AI clients | [`baukit-mcp`](rust/crates/baukit-mcp) | none | [Remote MCP](docs/guide/mcp.md) |
| Suites of linked products and entitlements | [`baukit-suite`](rust/crates/baukit-suite), [`baukit-events`](rust/crates/baukit-events) | [`@baukit/suite-client`](typescript/packages/suite-client), [`@baukit/events`](typescript/packages/events), [`@baukit/purchases-core`](typescript/packages/purchases-core), [`@baukit/purchases-revenuecat`](typescript/packages/purchases-revenuecat) | [Suite and platform](docs/guide/suite-and-platform.md) |
| Third-party integrations | [`baukit-integrations`](rust/crates/baukit-integrations), [`baukit-credential-vault`](rust/crates/baukit-credential-vault) | [`@baukit/integrations-client`](typescript/packages/integrations-client) | [Integrations](docs/guide/integrations.md) |
| Push and local notifications | [`baukit-push`](rust/crates/baukit-push) | [`@baukit/notifications-core`](typescript/packages/notifications-core), [`@baukit/notifications-expo`](typescript/packages/notifications-expo) | [Push and notifications](docs/guide/push-and-notifications.md) |
| Erasure and data-subject requests | [`baukit-erasure`](rust/crates/baukit-erasure), [`baukit-data-subject`](rust/crates/baukit-data-subject) | [`@baukit/analytics-core`](typescript/packages/analytics-core) | [Privacy](docs/guide/privacy.md) |
| Outbound HTTP and rate limits | [`baukit-egress`](rust/crates/baukit-egress), [`baukit-resilience`](rust/crates/baukit-resilience), [`baukit-ratelimit`](rust/crates/baukit-ratelimit) | none | [Outbound HTTP and limits](docs/guide/outbound-and-limits.md) |
| Web and native UI foundations | none | [`@baukit/ui-tokens`](typescript/packages/ui-tokens), [`@baukit/a11y-core`](typescript/packages/a11y-core), [`@baukit/navigation`](typescript/packages/navigation), [`@baukit/localization-core`](typescript/packages/localization-core), [`@baukit/preferences-core`](typescript/packages/preferences-core), [`@baukit/pwa-web`](typescript/packages/pwa-web), [`@baukit/csp`](typescript/packages/csp), [`@baukit/type-guards`](typescript/packages/type-guards), [`@baukit/eslint-config`](typescript/packages/eslint-config), [`@baukit/analytics-posthog-web`](typescript/packages/analytics-posthog-web), [`@baukit/analytics-posthog-native`](typescript/packages/analytics-posthog-native) | [Frontend](docs/guide/frontend.md) |
| Logs, traces and metrics | [`baukit-telemetry`](rust/crates/baukit-telemetry) | none | [Observability](docs/guide/observability.md) |
| Tests and conformance suites | [`baukit-test`](rust/crates/baukit-test) | none | [Quality gates](docs/guide/quality-gates.md) |

Every package ships ES modules. The Rust and TypeScript halves are written against the same contracts, and in the case of the event envelope both test suites read the same `fixtures/events/event-envelope-v1.json`. Drift between the browser and the server shows up as a failing test rather than a bad payload in production.

## How it stays correct

The generated product isn't a snapshot that rots. On every push CI scaffolds fresh products in each fixture flavor that [CLAUDE.md](CLAUDE.md) lists, then runs `cargo fmt --check`, `clippy -D warnings`, the full test suite with the Docker-backed tests, and an OpenAPI drift check against them. Template code is linted exactly like committed code. `sh scripts/ci-gate.sh` runs the same jobs locally in parallel lanes.

Each product inherits security suites from `baukit-test`. The generated `tests/conformance.rs` calls `assert_auth_negative_suite`, which sends forged, expired and wrong-issuer tokens to every operation in the OpenAPI document, and `assert_cross_tenant_denied`, which fails when a second principal can read the first one's data. `assert_ops_router_conformance` and `assert_metrics_conformance` pin the health and metrics contracts.

Tests check the inputs someone thought of, and some bugs live outside them. So baukit picks one tool per class of property, and every tool runs as a gate step ([ADR 0027](docs/adr/0027-verification-strategy.md)):

- Kani proves that pure kernels such as the egress address classifier and the rate-limit bucket hold for every input within the bounds each harness states.
- Verus proves twins of the HLC clock, the token bucket and the pull-page merge over traces of any length, and equivalence tests tie each twin to production code.
- TLA+ models in `models/` check protocols such as the job lease, the erasure saga and the sync pull over every interleaving. A `_current` config reproduces the old bug and must still fail.
- cargo-fuzz runs every decoder of untrusted bytes in `rust/fuzz`, such as JWT, webhook and suite ticket parsing, and fails on any crash.
- Generated vectors in `fixtures/` make the Rust and TypeScript halves agree case by case.

A proof only covers what its harness or model states. [Verification targets](docs/verification.md) lists every harness with its bounds and the known gaps, and the [security guide](docs/guide/security.md) says what a product still owns.

## Deploy and operate

The repo ships a [Helm chart](deploy/chart), [Grafana dashboards, alerts and recording rules](deploy/observability) matched to the metric names above, and an [nginx njs verifier for signed media grants](deploy/media-grants) that checks the same vectors as `baukit-core`'s signer. [`deploy/platform`](deploy/platform) runs the whole platform on a local k3d cluster, and [runbooks](docs/runbooks) cover every alert. See [deploying](docs/guide/deploying.md) and [local platform](docs/guide/local-platform.md).

## Documentation

- The [guide](docs/guide/README.md) walks through one task per page.
- [Contracts](docs/platform) hold the rules a product must follow or can rely on.
- [ADRs](docs/adr/README.md) explain why the architecture is the way it is.
- [Migrations](docs/migrations/README.md) say what changes for each breaking release.
- [Runbooks](docs/runbooks) say what to do when an alert fires.
- The [agent skills](agent-skills/README.md) teach Claude Code and Codex the conventions for product work.

[`docs/README.md`](docs/README.md) maps the rest.

## Working on baukit itself

Three workspaces, no root workspace, so always pass a manifest path.

```text
rust/        library crates, plus rust/fuzz and rust/verus
typescript/  npm packages, pnpm + Turborepo
cli/         the baukit binary
templates/   what the CLI renders
models/      TLA+ models
examples/    minimal-api and the Expo conformance apps
```

```bash
make toolchain          # mise: Node 26.10.0, Temurin 25.0.4.1+1, Rust 1.100.0, Kani, Verus and the linters
mise exec -- make ci    # fmt, clippy, tests, the TypeScript gates and make verify
mise exec -- sh scripts/ci-gate.sh    # every CI job in parallel lanes
```

`make ci` ends with `make verify`, which runs Kani, the TLA+ models, the vector check, the migration checks and the workflow audit, so it needs Docker and Java. Mise installs Corepack 0.36.0 with Node. Corepack reads the pnpm pin in `typescript/package.json`. Docker and the Android SDK stay system dependencies. iOS work needs xtool with a Darwin SDK you generate from a manually downloaded `Xcode.xip`, and Linux has no iOS simulator. [Verification](docs/verification.md) and [releasing](docs/releasing.md) cover the rest.

## Status and license

Every crate publishes to crates.io and every `@baukit/*` package to npm. The CLI installs from a Git tag. Crates, packages and templates move together under one `vX.Y.Z` tag.

MIT. See [LICENSE](LICENSE).

The [baukit jingle](docs/JINGLE.md) has English and German lyrics, video storyboards and reproducible ElevenLabs generation commands.
