# Dependency compatibility matrix

**Status:** Draft for adoption (Phase 0 deliverable)
**Home:** moves to the baukit repository; updated by the release train, not by hand-edits in products.
**Decisions behind it:** [platform analysis, sections 1.3 and 5.4](../shared-application-platform-analysis.md).

This table records what the shared baseline is **tested against**. Renovate keeps individual products moving; baukit guarantees compatibility only with the versions listed here. Version cells reflect the review-time state of the three projects and must be re-verified against the lockfiles when the baukit repository is created.

## Toolchain

| Tool | Tested baseline | Notes |
|---|---|---|
| Rust | MSRV per [conventions](./baukit-conventions.md) | CI-enforced |
| Node | current LTS | pinned via `.nvmrc`/engines |
| pnpm | 11 | pinned via `packageManager` |
| Turbo | 2 | |

## Backend (Rust)

| Responsibility | Dependency | Tested baseline | Notes |
|---|---|---|---|
| Async runtime | Tokio | 1.x (latest at train cut) | |
| HTTP | Axum | 0.8 | Tower / Tower HTTP at Axum-compatible versions |
| Persistence | SQLx | 0.9 | PostgreSQL, `runtime-tokio`, rustls |
| API description | Utoipa + utoipa-axum | 5 | |
| Traces | OpenTelemetry + tracing-opentelemetry | 0.32 + matching | upgrade only as a matched set |
| Metrics | metrics + metrics-exporter-prometheus | latest compatible | one recorder per process |
| Logging | tracing + tracing-subscriber | latest compatible | |
| Configuration | config + dotenvy | chosen loader (analysis §4.1) | Figment is not supported by the shared kit |
| Outbound HTTP | reqwest | latest, rustls | |
| Auth | jsonwebtoken + JWKS | latest | Keycloak default; Clerk/WorkOS adapters |
| Integration tests | testcontainers | latest | |

## Frontend (TypeScript)

| Responsibility | Dependency | Tested baseline | Notes |
|---|---|---|---|
| Mobile runtime | Expo SDK | 57 (RN 0.86, React 19.2) | React/RN versions follow the Expo SDK, verified with Expo Doctor |
| Mobile navigation | Expo Router | SDK 57 line | |
| Remote state | TanStack Query | 5 | |
| Web routing | TanStack Router | current v1 | re-verify TanStack Start status separately |
| Local state | Zustand | 5 | |
| Web build | Vite | 8 | |
| Styling | Tailwind CSS | 4 | |
| Web persistence | Dexie | latest 4.x | only when offline is enabled |
| Web e2e | Playwright | latest | |
| Native e2e | Maestro | latest | |

## Update rules

- The matrix changes only through a release-train PR in the baukit repository that runs the full test suite (unit, conformance, generated-fixture matrix) against the new versions.
- Renovate proposes updates into baukit; products receive them by upgrading their baukit version, not by diverging individually.
- The upgrade-sensitive sets (OpenTelemetry crates; Expo/React/React Native) are always updated as grouped PRs.
