# baukit

**baukit** is a nod to the German word *Baukasten*: a modular construction kit. It is a local-first application foundation that extracts the shared runtime, configuration, HTTP, operations, telemetry, testing, and frontend contracts used across independently buildable Rust-backed products without becoming a large framework or product-code monorepo.

## Repository layout

```text
baukit/
├── rust/
│   ├── Cargo.toml
│   └── crates/
│       ├── baukit-auth/
│       ├── baukit-config/
│       ├── baukit-core/
│       ├── baukit-credential-vault/
│       ├── baukit-events/
│       ├── baukit-http/
│       ├── baukit-integrations/
│       ├── baukit-jobs/
│       ├── baukit-openapi/
│       ├── baukit-ops/
│       ├── baukit-push/
│       ├── baukit-ratelimit/
│       ├── baukit-runtime/
│       ├── baukit-sync/
│       ├── baukit-telemetry/
│       └── baukit-test/
├── typescript/
│   └── packages/
│       ├── a11y-core/
│       ├── analytics-core/
│       ├── analytics-posthog-web/
│       ├── analytics-posthog-native/
│       ├── api-runtime/
│       ├── auth-native/
│       ├── auth-web/
│       ├── data-contracts/
│       ├── data-contracts-dexie/
│       ├── data-contracts-expo-sqlite/
│       ├── events/
│       ├── localization-core/
│       ├── preferences-core/
│       ├── pwa-web/
│       ├── sync-client/
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
| `baukit-auth` | OIDC verification, personal access tokens, and Axum principal extraction. |
| `baukit-config` | Layered configuration, validation, standard settings, and secret wrappers. |
| `baukit-core` | Dependency-light shared vocabulary for the other crates. |
| `baukit-credential-vault` | Versioned AES-256-GCM encryption and a storage-neutral port for third-party credentials. |
| `baukit-events` | Versioned product event envelope, stable validation codes, and ingestion outcomes. |
| `baukit-http` | Shared Axum middleware, errors, traffic policy, keyset pagination, upstream retry classes, tracing, and HTTP metrics. |
| `baukit-integrations` | Provider connector contract for cursor-paged imports, verified webhooks, and connection health. |
| `baukit-jobs` | Durable PostgreSQL outbox storage and supervised worker execution. |
| `baukit-openapi` | Utoipa metadata, deterministic schema output, and drift checks. |
| `baukit-ops` | Separate liveness, readiness, metrics, and build-information endpoints. |
| `baukit-push` | Provider-neutral push delivery port with an Expo ticket and receipt adapter. |
| `baukit-ratelimit` | Redis-backed identity and client-IP token-bucket rate limiting. |
| `baukit-runtime` | Process lifecycle, service identity, task supervision, and listener composition. |
| `baukit-sync` | Per-owner revision allocation and the syncable-table column convention. |
| `baukit-telemetry` | Structured logging, OpenTelemetry traces, and Prometheus metrics. |
| `baukit-test` | Integration fixtures and operational, API, and port conformance tests. |
| `@baukit/a11y-core` | Overlay focus, inert, announcements, and reduced motion for web and React Native. |
| `@baukit/analytics-core` | Provider-neutral typed analytics, consent, identity, and privacy controls. |
| `@baukit/analytics-posthog-web` | PostHog transport for web applications. |
| `@baukit/analytics-posthog-native` | PostHog transport for native applications. |
| `@baukit/api-runtime` | Generated-client runtime policy for auth, tracing, errors, retries, and tests. |
| `@baukit/auth-native` | Provider-neutral native OIDC lifecycle with an optional Expo adapter. |
| `@baukit/auth-web` | Provider-neutral browser OIDC authorization-code and PKCE client. |
| `@baukit/data-contracts` | Storage contracts, adapter helpers, and reusable conformance suites. |
| `@baukit/data-contracts-dexie` | Dexie storage adapters with fake and real-browser transaction conformance. |
| `@baukit/data-contracts-expo-sqlite` | Expo SQLite implementation of the shared storage contracts. |
| `@baukit/events` | Zod schemas and validation for the shared product event envelope. |
| `@baukit/localization-core` | Locale resolution, formatting policy, and timezone-safe civil-date arithmetic. |
| `@baukit/preferences-core` | Identity-guarded preference controller and repository store. |
| `@baukit/pwa-web` | Request classification and cache-strategy execution for a product-owned service worker. |
| `@baukit/sync-client` | Sync scheduler, transport, status store, and push-batch ranking. |
| `@baukit/ui-tokens` | Cross-platform design-token schema, validation, generated outputs, and the `no-raw-color` eslint rule. |

## Agent skills

Portable workflows for scaffolding, endpoint work, [integration work](agent-skills/skills/baukit-add-integration/SKILL.md), [accessibility](agent-skills/skills/baukit-accessibility/SKILL.md), localization, mirrored domain logic, upgrades, and observability live in [`agent-skills/`](agent-skills/). Install them with `./agent-skills/install.sh --target <product-dir> [--claude] [--codex] [--copy]` or `make install-skills TARGET=<product-dir>`; without harness flags, the installer uses existing `.claude/` and `.agents/` parents.

## Platform contracts and recipes

The platform guides cover [local-data ownership](docs/platform/local-data-ownership-contract.md), [offline readiness](docs/platform/offline-readiness-contract.md), [integration reliability](docs/platform/integration-reliability.md), [native quality gates](docs/platform/native-quality-gates.md), [accessibility](docs/platform/accessibility-contract.md), [localization](docs/platform/localization-contract.md), [onboarding](docs/platform/onboarding-recipe.md), and [mirrored Rust/TypeScript domain logic](docs/platform/mirrored-domain-logic.md).

## Toolchains and releases

The repository pins Node 24, Temurin 21, and Rust 1.97.1 in `mise.toml`.
Install the local toolchain and run commands inside it with:

```bash
make toolchain
mise exec -- make ci
```

Corepack reads pnpm 11.18.0 from `typescript/package.json`. Docker and the
Android SDK remain system dependencies. The Rust workspaces use edition 2024
and keep Rust 1.95 as their MSRV, which CI checks separately.

Swift and xtool are pinned for iOS work in mobile products. xtool builds
SwiftPM packages against a Darwin SDK that `xtool setup` generates from an
`Xcode.xip` downloaded manually with an Apple ID, but Linux has no iOS
simulator.

Baukit is MIT licensed. The `@baukit/*` TypeScript packages are published to npm and the sixteen library crates to crates.io; the `baukit` CLI is installed from a Git tag. All components remain on one `0.x` release train. Before 1.0, breaking changes bump the minor version; Rust releases use release-plz and TypeScript releases use Changesets. The latest minor of the latest release train is the only supported line, with no pre-1.0 backports.

## Releases

Crates, `@baukit/*` packages, and templates advance together under one
`vX.Y.Z` tag. Each train publishes the `@baukit/*` packages to npm and the
library crates to crates.io. Add TypeScript release notes with
`pnpm --dir typescript changeset`, then follow the coordinated cut, validation,
tagging, and product-consumption steps in [the release runbook](docs/releasing.md).
