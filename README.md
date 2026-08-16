# baukit

**baukit** is a nod to the German word *Baukasten*: a modular construction kit. It is a private-first application foundation that extracts the shared runtime, configuration, HTTP, operations, telemetry, testing, and frontend contracts used across independently buildable Rust-backed products without becoming a large framework or product-code monorepo.

## Repository layout

```text
baukit/
├── rust/
│   ├── Cargo.toml
│   └── crates/
│       ├── baukit-runtime/
│       ├── baukit-config/
│       ├── baukit-http/
│       ├── baukit-ops/
│       ├── baukit-telemetry/
│       ├── baukit-openapi/
│       └── baukit-test/
├── typescript/
│   └── packages/
│       ├── analytics-core/
│       ├── analytics-posthog-web/
│       ├── analytics-posthog-native/
│       ├── api-runtime/
│       ├── data-contracts/
│       └── ui-tokens/
├── cli/
├── templates/
├── deploy/
│   ├── chart/
│   └── observability/{dashboards,alerts,recording-rules}/
├── agent-skills/
└── examples/
```

## Components

| Component | Responsibility |
|---|---|
| `baukit-runtime` | Process lifecycle, service identity, task supervision, and listener composition. |
| `baukit-config` | Layered configuration, validation, standard settings, and secret wrappers. |
| `baukit-http` | Shared Axum middleware, errors, traffic policy, tracing, and HTTP metrics. |
| `baukit-ops` | Separate liveness, readiness, metrics, and build-information endpoints. |
| `baukit-telemetry` | Structured logging, OpenTelemetry traces, and Prometheus metrics. |
| `baukit-openapi` | Utoipa metadata, deterministic schema output, and drift checks. |
| `baukit-test` | Integration fixtures and operational, API, and port conformance tests. |
| `@baukit/analytics-core` | Provider-neutral typed analytics, consent, identity, and privacy controls. |
| `@baukit/analytics-posthog-web` | PostHog transport for web applications. |
| `@baukit/analytics-posthog-native` | PostHog transport for native applications. |
| `@baukit/api-runtime` | Generated-client runtime policy for auth, tracing, errors, retries, and tests. |
| `@baukit/data-contracts` | Storage contracts, adapter helpers, and reusable conformance suites. |
| `@baukit/ui-tokens` | Cross-platform design-token schema, validation, and generated outputs. |

## Agent skills

Portable workflows for scaffolding, endpoint work, localization, mirrored domain logic, upgrades, and observability live in [`agent-skills/`](agent-skills/). Install them with `./agent-skills/install.sh --target <product-dir> [--claude] [--codex] [--copy]` or `make install-skills TARGET=<product-dir>`; without harness flags, the installer uses existing `.claude/` and `.agents/` parents.

## Platform contracts and recipes

The platform guides cover [local-data ownership](docs/platform/local-data-ownership-contract.md), [offline readiness](docs/platform/offline-readiness-contract.md), [localization](docs/platform/localization-contract.md), [onboarding](docs/platform/onboarding-recipe.md), and [mirrored Rust/TypeScript domain logic](docs/platform/mirrored-domain-logic.md).

## Toolchains and releases

The Rust workspace uses edition 2024 and has an MSRV of Rust 1.95, calculated as stable Rust 1.97 minus two minor releases. The TypeScript workspace pins pnpm 11.18.0; full TypeScript tooling arrives in a later wave.

Everything is private, proprietary, and unpublished for now, so this repository intentionally has no license files. All components remain on one `0.x` release train. Before 1.0, breaking changes bump the minor version; Rust releases will use release-plz, TypeScript releases will use Changesets, and products will consume private git dependencies pinned to release tags. The latest minor of the latest release train is the only supported line, with no pre-1.0 backports.

## Releases

Crates, `@baukit/*` packages, and templates advance together under one
`baukit-vX.Y.Z` tag; nothing is published to crates.io or npm while the
repository is private. Add TypeScript release notes with
`pnpm --dir typescript changeset`, then follow the coordinated cut, validation,
tagging, and product-consumption steps in [the release runbook](docs/releasing.md).
