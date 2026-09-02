# baukit

[![CI](https://github.com/PatrickKoss/baukit/actions/workflows/ci.yml/badge.svg)](https://github.com/PatrickKoss/baukit/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/baukit-runtime.svg?label=crates.io)](https://crates.io/crates/baukit-runtime)
[![npm](https://img.shields.io/npm/v/@baukit/api-runtime.svg?label=npm)](https://www.npmjs.com/package/@baukit/api-runtime)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

The boring 80% of a Rust backend and its TypeScript clients, already built and already tested.

*Baukit* is German for a construction kit, and that is the whole idea. Sixteen Rust crates and seventeen npm packages, each small enough to adopt on its own, plus a CLI that wires them into a working product when you want the whole thing.

> [!WARNING]
> **Under heavy development. Expect breaking changes.**
> Every component is on one `0.x` train and minor bumps break API before 1.0. Only the latest minor is supported, and there are no backports. **Pull requests are not accepted yet.** Issues and discussions are welcome.

## The problem

Every new service starts with the same three weeks. Graceful shutdown that actually drains. Health endpoints that a load balancer can distinguish from readiness. Config from files and environment with real validation. An error envelope your frontend can branch on. Trace IDs that survive the hop from browser to backend. Prometheus metrics named consistently enough that one dashboard works for every service.

None of it is hard. All of it is fiddly, all of it gets done slightly differently each time, and all of it is what breaks at 3am.

Baukit is that work, extracted from real products, with the tests attached.

## Quickstart

Scaffold a product:

```bash
cargo install --git https://github.com/PatrickKoss/baukit --tag v0.1.2 --locked baukit-cli
baukit new orders --backend --web
cd orders && make check
```

Add `--quality strict` to generate capability-specific coverage, migration, OpenAPI, browser, image, and native gates. The generated `scripts/quality-gate.sh` runs the same checks locally. Standard remains the default.

You get a hexagonal Rust backend (`orders-domain`, `-ports`, `-services`, `-api`, `-postgres`, `-bin`), a Vite React app with a typed client generated from the backend's OpenAPI schema, Docker builds, a Helm values file, CI, and conformance tests that fail if the health and metrics contracts drift.

Generated products pull Baukit from crates.io and npm by pinned version, so nothing here needs repository access or an SSH key. Point the CLI at a local checkout with `--baukit-path rust` when you want path and `file:` dependencies instead.

Or add one crate to a service you already have:

```toml
[dependencies]
baukit-http = "0.1"
baukit-ops = "0.1"
```

## What it looks like running

The [`examples/minimal-api`](examples/minimal-api) notes service is 433 lines of Rust, comments included. Start it:

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
          "request_id":"0f9bead5-9dd2-4f45-b876-0e12e9d663a3",
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
{"service_name":"minimal-api-api","version":"0.1.0","commit":"unknown","rust_version":"1.97.1"}
```

`/healthz` says the process is alive, `/readyz` says it should receive traffic, and they are genuinely different during a deploy. When shutdown starts, the traffic gate flips `/readyz` to failing while in-flight requests finish draining, which is the difference between a rolling deploy and a handful of dropped connections.

Metrics come out already named the way the shipped Grafana dashboards expect: `http_requests_total`, `http_request_duration_seconds`, `http_requests_in_flight`, `build_info`. A linter in CI keeps them that way.

## Pick what you need

Nothing here depends on the CLI, and no crate drags in the rest. Take the error envelope and skip the job queue.

### Rust

| Crate | What it gives you |
|---|---|
| [`baukit-runtime`](rust/crates/baukit-runtime) | Process lifecycle, graceful drain, supervised background tasks, listener composition |
| [`baukit-config`](rust/crates/baukit-config) | Layered file and environment config, validation, secret wrappers that refuse to print |
| [`baukit-http`](rust/crates/baukit-http) | Axum middleware, the error envelope, keyset pagination, upstream retry classification, CORS |
| [`baukit-ops`](rust/crates/baukit-ops) | Separate liveness, readiness, metrics, and build-info endpoints |
| [`baukit-telemetry`](rust/crates/baukit-telemetry) | Structured logs, OpenTelemetry traces, Prometheus metrics from one builder |
| [`baukit-auth`](rust/crates/baukit-auth) | OIDC verification, personal access tokens, Axum principal extraction |
| [`baukit-jobs`](rust/crates/baukit-jobs) | Durable PostgreSQL outbox and supervised workers |
| [`baukit-openapi`](rust/crates/baukit-openapi) | Utoipa metadata, deterministic schema output, drift checks |
| [`baukit-ratelimit`](rust/crates/baukit-ratelimit) | Redis token buckets keyed by identity or client IP |
| [`baukit-sync`](rust/crates/baukit-sync) | Per-owner revision allocation for offline clients |
| [`baukit-events`](rust/crates/baukit-events) | Versioned event envelope with stable validation codes |
| [`baukit-integrations`](rust/crates/baukit-integrations) | Connector contract for cursor-paged imports, verified webhooks, health |
| [`baukit-credential-vault`](rust/crates/baukit-credential-vault) | Versioned AES-256-GCM encryption behind a storage-neutral port |
| [`baukit-push`](rust/crates/baukit-push) | Provider-neutral push delivery with an Expo adapter |
| [`baukit-core`](rust/crates/baukit-core) | Dependency-light vocabulary shared by the others |
| [`baukit-test`](rust/crates/baukit-test) | Docker PostgreSQL and Redis fixtures, a mock OIDC issuer, conformance suites |

### TypeScript

| Package | What it gives you |
|---|---|
| [`@baukit/api-runtime`](typescript/packages/api-runtime) | Auth and request-ID headers, trace propagation, normalized errors, safe retries |
| [`@baukit/auth-web`](typescript/packages/auth-web) · [`-native`](typescript/packages/auth-native) | OIDC authorization code with S256 PKCE, refresh rotation |
| [`@baukit/data-contracts`](typescript/packages/data-contracts) | Storage contracts plus conformance suites you run against your adapter |
| [`@baukit/data-contracts-dexie`](typescript/packages/data-contracts-dexie) · [`-expo-sqlite`](typescript/packages/data-contracts-expo-sqlite) | IndexedDB and Expo SQLite adapters, verified in a real browser and on a real Android device in CI |
| [`@baukit/integrations-client`](typescript/packages/integrations-client) | Connection health, OAuth session coordination, provider registry |
| [`@baukit/sync-client`](typescript/packages/sync-client) | Sync scheduling, transport, status store, push-batch ordering |
| [`@baukit/ui-tokens`](typescript/packages/ui-tokens) | Design-token schema, contrast checker, CSS and React Native compilers, a `no-raw-color` eslint rule |
| [`@baukit/a11y-core`](typescript/packages/a11y-core) | Focus traps, inert backgrounds, announcements, reduced motion, on both web and native |
| [`@baukit/analytics-core`](typescript/packages/analytics-core) | Typed events with consent and privacy controls, PostHog transports for [web](typescript/packages/analytics-posthog-web) and [native](typescript/packages/analytics-posthog-native) |
| [`@baukit/localization-core`](typescript/packages/localization-core) | Locale resolution, formatting policy, timezone-safe civil-date math |
| [`@baukit/preferences-core`](typescript/packages/preferences-core) | Identity-guarded preference controller |
| [`@baukit/pwa-web`](typescript/packages/pwa-web) | Request classification and cache strategies for a service worker you own |
| [`@baukit/events`](typescript/packages/events) | Zod schemas for the same envelope `baukit-events` speaks |

The Rust and TypeScript halves are written against the same contracts, and in the case of the event envelope both test suites read the same `fixtures/events/event-envelope-v1.json`. Drift between the browser and the server shows up as a failing test rather than a bad payload in production.

## Why it holds up

The generated product isn't a snapshot that rots. CI scaffolds a fresh one on every push and runs `cargo fmt --check`, `clippy -D warnings`, the full test suite, and an OpenAPI drift check against it, in backend, web, and combined flavors. Template code is linted exactly like committed code.

The rest of the gate is similar. MSRV stays pinned at Rust 1.95 and CI checks it separately. `cargo deny` runs on advisories and licenses. A metric-name linter keeps dashboards and alerts honest. Dexie conformance runs in a real browser and the Expo SQLite adapter runs on a real Android device. Nobody skips the Docker-backed integration tests.

The repo also ships a [Helm chart](deploy/chart) and [Grafana dashboards, alerts, and recording rules](deploy/observability) matched to the metric names above. The [agent skills](agent-skills/) teach Claude Code and Codex the conventions for adding an endpoint, an integration, or a locale.

## Documentation

Contracts and recipes live in [`docs/platform/`](docs/platform): [local-data ownership](docs/platform/local-data-ownership-contract.md), [offline readiness](docs/platform/offline-readiness-contract.md), [integration reliability](docs/platform/integration-reliability.md), [accessibility](docs/platform/accessibility-contract.md), [localization](docs/platform/localization-contract.md), [native quality gates](docs/platform/native-quality-gates.md), [telemetry](docs/platform/telemetry-spec.md), [onboarding](docs/platform/onboarding-recipe.md), and [mirrored Rust/TypeScript domain logic](docs/platform/mirrored-domain-logic.md). Release process is in [`docs/releasing.md`](docs/releasing.md).

## Working on baukit itself

Three workspaces, no root workspace, so always pass a manifest path.

```text
rust/        16 library crates
typescript/  16 packages, pnpm + Turborepo
cli/         the baukit binary
templates/   what the CLI renders
examples/    minimal-api, expo-sqlite-conformance
```

```bash
make toolchain          # mise: Node 24, Temurin 21, Rust 1.97.1
mise exec -- make ci    # fmt, clippy, tests, and the TypeScript gates
```

Corepack picks up pnpm 11.18.0 from `typescript/package.json`. Docker and the Android SDK stay system dependencies. iOS work needs xtool with a Darwin SDK you generate from a manually downloaded `Xcode.xip`, and Linux has no iOS simulator.

## Status and license

The `@baukit/*` packages are on npm and the sixteen library crates are on crates.io at `0.1.2`. The CLI installs from a Git tag. Crates, packages, and templates move together under one `vX.Y.Z` tag.

MIT. See [LICENSE](LICENSE).
