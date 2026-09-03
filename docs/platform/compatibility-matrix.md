# Dependency compatibility matrix

**Status:** Adopted.
**Home:** this repository; updated by the release train, not by hand-edits in products.

This table records what the shared baseline is **tested against**. Renovate keeps individual products moving; baukit guarantees compatibility only with the versions listed here. Version cells reflect the review-time state of the three projects and must be re-verified against the lockfiles when the baukit repository is created.

Last verified release train: `v0.2.1` (typed product connectors and immutable
connection-state overlays in `@baukit/integrations-client`, atomic fixed-window
amount release in `baukit-ratelimit`, manifest-aware `baukit doctor` checks,
and Metro bundling for product-root limits policies; backend, web, mobile,
combined, and authenticated generated fixtures, native Android compile, real
Expo SQLite conformance, browser Dexie conformance, Docker-backed integration
tests, the MSRV check, and the complete local CI-equivalent gates were
verified).

## Toolchain

| Tool | Tested baseline | Notes |
|---|---|---|
| Rust | 1.95.0 MSRV | CI-enforced with Rust 1.95; train cut on stable 1.97.1 |
| Node | 24 (v24.19.0 test host) | pinned via `mise.toml`, `typescript/.nvmrc`, and package engines |
| Java | Temurin 21 (21.0.12.1 test host) | pinned via `mise.toml`; used by Android builds |
| Swift | 6.3.3 | pinned via `mise.toml`; compiler version checked on Linux |
| xtool | 1.17.0 | pinned via `mise.toml`; version checked on Linux, without a Darwin SDK or simulator |
| pnpm | 11.18.0 | pinned via `packageManager` |
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
| Integration tests | testcontainers | latest | `baukit-test` pins `postgres:18-alpine`; templates and smoke deploys use the same image |
| Sync revisions | `baukit-sync` | 0.2.1 | Per-owner revision allocation, locking revision reads, the syncable-table column convention, and a `user_id` to `owner_id` migration; SQLx 0.9, PostgreSQL. |
| Provider connectors | `baukit-integrations` | 0.2.1 | Contract-only connector port, cursor-paged pages, and `baukit-http` retry classes; no SQLx, no HTTP client. |

## Cross-runtime contracts

| Responsibility | Rust and TypeScript packages | Tested baseline | Notes |
|---|---|---|---|
| Suite event envelope | `baukit-events` and `@baukit/events` | 0.2.1 | Version 1 envelope, stable validation codes, seven-day replay boundary, and one fixture corpus exercised in both languages. |

## Frontend (TypeScript)

| Responsibility | Dependency | Tested baseline | Notes |
|---|---|---|---|
| Mobile runtime | Expo SDK | 57.0.11 (RN 0.86.2, React 19.2.8) | React/RN versions follow the Expo SDK, verified with Expo Doctor |
| Mobile navigation | Expo Router | 57.0.15 | Generated mobile template baseline with Expo Router and `Stack.Protected` in the auth overlay; `react-native-screens` 4.26.2, `react-native-safe-area-context` 5.7.0, `react-native-reanimated` 4.5.1, `react-native-worklets` 0.10.1, and `react-native-gesture-handler` 2.32.0. |
| Remote state | TanStack Query | 5 | |
| Web routing | TanStack Router | current v1 | re-verify TanStack Start status separately |
| Local state | Zustand | 5 | |
| Accessibility behavior | `@baukit/a11y-core` | 0.2.1 | Overlay focus, inert, announcements, reduced motion. React peer range is `^19.2.0`; React Native is optional, and a plain web app imports `@baukit/a11y-core/web` instead. |
| Localization behavior | `@baukit/localization-core` | 0.2.1 | Locale resolution, catalog key comparison, stable-code localization, and timezone-safe civil-date arithmetic. |
| Preference behavior | `@baukit/preferences-core` | 0.2.1 | Identity guard and repository store, with `null` repository records treated as missing. |
| Provider registry | `@baukit/integrations-client` | 0.2.1 | Typed product connectors, stable registration order, and immutable connection-state overlays. |
| Client sync primitives | `@baukit/sync-client` | 0.2.1 | Scheduler, request-function and HTTP transports, status store, and push-batch ranking. The optional `@baukit/sync-client/expo` entry uses Expo Network 57.0.1 and React Native 0.86.2; the root entry has no runtime dependencies and no React. |
| PWA cache strategy | `@baukit/pwa-web` | 0.2.1 | ESM and CJS builds, request classification, `navigationFallback`, and strategy execution for a product-owned service worker; no dependencies and no service-worker globals. |
| Web build | Vite | 8.2.1 | |
| Styling | Tailwind CSS | 4 | |
| Web persistence | `@baukit/data-contracts-dexie` / Dexie | 4.4.5 | only when offline is enabled; Chromium and WebKit conformance-tested |
| Native scoped persistence digest | `expo-crypto` | 57.0.1 | Expo adapter injected into the identity-scoping contract |
| Native accessibility lint | `eslint-plugin-react-native-a11y` + `@eslint/compat` | 3.5.1 + 2.1.0 | Generated mobile template lint baseline |
| Web accessibility checks | axe-core | 4.13.0 | Serious/critical jsdom scan seam; contrast remains a real-browser check |
| Web e2e | Playwright | 1.62.1 | Chromium 151.0.7922.34 (revision 1234) and WebKit 26.5 (revision 2336) |
| Android native compile | Expo prebuild + Gradle | API 36, build-tools 36.0.0, Java 21 | Blocking for relevant generated-product and Baukit fixture changes |
| Native e2e | Maestro | latest | Configurable for product-owned critical paths; scheduled/manual, not part of the universal pull-request promise |
| iOS native compile | Xcode + iOS Simulator | macOS runner | Scheduled/manual; Linux is recorded as blocked, never as a passing skip |

## Update rules

- The matrix changes only through a release-train PR in the baukit repository that runs the full test suite (unit, conformance, generated-fixture matrix) against the new versions.
- Renovate proposes updates into baukit; products receive them by upgrading their baukit version, not by diverging individually.
- The upgrade-sensitive sets (OpenTelemetry crates; Expo/React/React Native) are always updated as grouped PRs.
