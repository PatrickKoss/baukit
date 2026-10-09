# Dependency compatibility matrix

**Status:** Adopted.
**Home:** this repository; updated by the release train, not by hand-edits in products.

This table records the tested baseline. The committed lockfiles record exact resolutions. Third-party Rust manifests use caret requirements with tested minimums, so products can take compatible updates in their own lockfiles without a Baukit release. A requirement change must pass tests at the direct minimums and the newest compatible resolution. Internal `baukit-*` requirements stay exact because the crates release together.

Last verified release train: `v0.10.5` (doctor checks backend product identities and the OIDC admin realm only in the backend
Cargo workspace, so separate services such as agents keep their own configuration names).

## Toolchain

| Tool | Tested baseline | Notes |
|---|---|---|
| Rust | 1.95.0 MSRV | CI-enforced with Rust 1.95; refresh uses stable 1.99.0 |
| Node | 26.10.0 | pinned via `mise.toml` and `typescript/.nvmrc`; Node types use major 26. Package engines still accept Node 24 and later. |
| Corepack | 0.36.0 | installed through mise locally and npm in CI; Node 26 does not bundle it |
| Java | Temurin 25.0.4.1+1 | `mise.toml` and every Android CI job pin the same build as `25.0.4+101.0.LTS`, the Adoptium API version. Generated builds and Expo conformance builds grant native access to Android's Prefab tool. |
| Swift | 6.4.0 | pinned via `mise.toml`; compiler version checked on Linux |
| xtool | 1.20.1 | pinned via `mise.toml`; version checked on Linux, without a Darwin SDK or simulator |
| pnpm | 12.9.1 | pinned via `packageManager`; fresh generated web, mobile, and MCP lockfiles pass `--frozen-lockfile` |
| Turbo | 2 | |

## Backend (Rust)

| Responsibility | Dependency | Tested baseline | Notes |
|---|---|---|---|
| Async runtime | Tokio | 1.x (latest at train cut) | |
| HTTP | Axum | 0.8 | Tower / Tower HTTP at Axum-compatible versions |
| Persistence | SQLx | 0.9 | PostgreSQL 18.6, `runtime-tokio`, rustls |
| Database | PostgreSQL | 18.6 | Alpine in tests and generated Compose; CNPG uses `18.6-system-trixie` for primary and restore clusters. |
| API description | Utoipa | 6 | |
| Traces | OpenTelemetry + tracing-opentelemetry | 0.33 + 0.34 | upgrade only as a matched set |
| Metrics | metrics + metrics-exporter-prometheus | latest compatible | one recorder per process |
| Logging | tracing + tracing-subscriber | latest compatible | |
| Configuration | config + dotenvy | chosen loader (analysis §4.1) | Figment is not supported by the shared kit |
| Outbound HTTP | reqwest | 0.13, rustls with the platform verifier | |
| Auth | aws-lc-rs + JWKS | latest | Keycloak OIDC default; Clerk session-token adapter with `azp` validation; WorkOS AuthKit adapter bound to `client_id`. `ApiTokenStore` returns `ApiTokenStoreError` since 0.3.0. |
| Local object storage | RustFS | 1.0.1 | [Latest stable release](https://github.com/rustfs/rustfs/releases/tag/1.0.1), pinned by multi-platform image digest. Local Loki, PostHog and CNPG use path-style S3 with region `us-east-1`. |
| Development identity provider | Keycloak | 26.8.0 | Generated `compose.yaml` image; `make dev` reconciles the development realm from `realm-policy.json`. |
| Integration tests | testcontainers | 0.28.0 | `baukit-test` uses `GenericImage` and no longer depends on testcontainers-modules, whose 0.15.0 release requires 0.27. `baukit-test` pins `postgres:18.6-alpine`; templates and smoke deploys use the same image. |
| Sync revisions | `baukit-sync` | 0.5.2 | Per-owner revision allocation, locking revision reads, tombstone purge horizons with a pull-cursor guard, the syncable-table column convention, and a `user_id` to `owner_id` migration. SQLx 0.9 and PostgreSQL behind the `sqlx-postgres` feature; the hybrid logical clock needs neither. |
| Provider connectors | `baukit-integrations` | 0.5.2 | Contract-only connector port, cursor-paged pages, and `baukit-http` retry classes; no SQLx, no HTTP client. |

## PostgreSQL 18 storage and compatibility

Docker Official Image containers mount `/var/lib/postgresql` and keep data at
`/var/lib/postgresql/18/docker`. CNPG manages its own PVC mounts. Recreate local
volumes and PVCs for the major-version change because no product is live.
Existing PostgreSQL data needs dump/restore or `pg_upgrade` if it must be kept.
See the [image storage contract](https://hub.docker.com/_/postgres#pgdata).

PostgreSQL 18 enables data checksums for new clusters and deprecates MD5
password authentication. Keep checksums enabled and use the default
`scram-sha-256` password encryption. SQLx 0.9 migrations and role setup are
checked against 18.6 in the Docker suites. Baukit migrations do not use implicit
generated columns, unlogged partitioned tables, or the removed statistics
columns. See the [PostgreSQL 18 release notes](https://www.postgresql.org/docs/18/release-18.html).

## Cross-runtime contracts

| Responsibility | Rust and TypeScript packages | Tested baseline | Notes |
|---|---|---|---|
| Suite event envelope | `baukit-events` and `@baukit/events` | 0.5.2 | Version 1 envelope, stable validation codes, seven-day replay boundary, and one fixture corpus exercised in both languages. |

## Frontend (TypeScript)

| Responsibility | Dependency | Tested baseline | Notes |
|---|---|---|---|
| Mobile runtime | Expo SDK | 57.0.27 (RN 0.86.3, React 19.2.3) | React/RN versions follow the Expo SDK, verified with Expo Doctor |
| Mobile navigation | Expo Router | 57.0.25 | Generated mobile template baseline with Expo Router and `Stack.Protected` in the auth overlay; `react-native-screens` 4.26.2, `react-native-safe-area-context` 5.7.0, `react-native-reanimated` 4.5.1, `react-native-worklets` 0.10.1, and `react-native-gesture-handler` 2.32.0. |
| Remote state | TanStack Query | 5 | |
| Web routing | TanStack Router | current v1 | re-verify TanStack Start status separately |
| Local state | Zustand | 5 | |
| Accessibility behavior | `@baukit/a11y-core` | 0.5.2 | Overlay focus, inert, announcements and reduced motion (both also on the `/web` entry). React peer range is `^19.2.0`; React Native is optional, and a plain web app imports `@baukit/a11y-core/web` instead. |
| Localization behavior | `@baukit/localization-core` | 0.5.2 | Locale resolution, catalog key comparison, stable-code localization, timezone-safe civil-date arithmetic, and local-time resolution with required gap and fold policies. |
| Local notification planning | `@baukit/notifications-core` | 0.5.2 | Zoned occurrence resolution through `resolveZonedLocalTime`, horizon filtering, deterministic keep, cancel, and schedule sets, and owner-scoped replacement over a platform port. No dependencies besides `@baukit/localization-core`. |
| Local notification delivery | `@baukit/notifications-expo` / `expo-notifications` | 0.5.2 / 57.0.21 | Optional adapter. Imports only types from `expo-notifications`; the product passes the module in. DATE triggers, per-item results, no cancel-all. |
| Preference behavior | `@baukit/preferences-core` | 0.5.2 | Identity guard and repository store, with `null` repository records treated as missing. |
| Node device authentication | `@baukit/auth-node` | 0.5.2 | Node 24 OIDC device authorization with S256 PKCE, bounded responses and timeouts, refresh rotation, and a locked local profile cache. Plain HTTP requires an explicit loopback-only development policy. The `/keycloak-testing` entry holds Keycloak e2e helpers. |
| Provider registry | `@baukit/integrations-client` | 0.5.2 | Typed product connectors, stable registration order, and immutable connection-state overlays. |
| Client sync primitives | `@baukit/sync-client` | 0.5.2 | Scheduler with optional retry, request-function and HTTP transports, status store, push-batch ranking, a persisted hybrid logical clock, and tombstone-horizon conformance. The optional `@baukit/sync-client/expo` entry uses Expo Network 57.0.2 and React Native 0.86.3; the root entry has no runtime dependencies and no React. |
| PWA cache strategy | `@baukit/pwa-web` | 0.5.2 | ESM and CJS builds, request classification, `navigationFallback`, and strategy execution for a product-owned service worker; no dependencies and no service-worker globals. |
| MCP server | `rmcp` | 3.5.1 | `--mcp --backend --auth oidc` generates Rust Streamable HTTP with resource OAuth and service ports. |
| Web build | Vite | 8.3.2 | |
| Styling | Tailwind CSS | 4 | |
| Web persistence | `@baukit/data-contracts-dexie` / Dexie | 4.4.6 | only when offline is enabled; Chromium and WebKit conformance-tested |
| Native scoped persistence digest | `expo-crypto` | 57.0.3 | Expo adapter injected into the identity-scoping contract |
| Native accessibility lint | `eslint-plugin-react-native-a11y` + `@eslint/compat` | 3.5.1 + 2.1.1 | Generated mobile template lint baseline on ESLint 10; the plugin declares an ESLint 8 peer, so the template allows ESLint 10 through `peerDependencyRules` |
| Web accessibility checks | axe-core | 4.13.0 | Serious/critical jsdom scan seam; contrast remains a real-browser check |
| Web e2e | Playwright | 1.63.0 | Chromium 153.0.8010.12 (revision 1243) and WebKit 26.6 (revision 2359) |
| Android native compile | Expo prebuild + Gradle | API 36, build-tools 36.0.0, Temurin 25.0.4.1+1 | Blocking for relevant generated-product and Baukit fixture changes |
| Native e2e | Maestro | latest | Configurable for product-owned critical paths; scheduled/manual, not part of the universal pull-request promise |
| iOS native compile | Xcode + iOS Simulator | macOS runner | Scheduled/manual; Linux is recorded as blocked, never as a passing skip |

## Dependencies kept on a compatible line

The table records retained dependency lines and the available upstream
releases.

| Dependency | Available stable release | Reason for the current line |
|---|---|---|
| TypeScript | 7.0.2 | typescript-eslint 8.71.0 accepts TypeScript below 6.1. |
| Expo SDK | 57.0.27 | SDK 57 is the latest stable SDK. Its native package versions remain together. |
| React Native | 0.87.1 | Expo SDK 57 uses 0.86.3, including the Babel and Jest presets. |
| React on mobile | 19.3.0 | Expo SDK 57 uses 19.2.3. Web uses 19.3.0. |
| Jest and @jest/globals | 30.5.2 | jest-expo 57 and the React Native Jest preset use Jest 29. Types use @types/jest 29.5.14. |
| Babel | 8.0.6 | The React Native Babel preset uses Babel 7 plugins. |
| test-renderer | 1.3.0 | Its React reconciler requires React 19.3. The Expo set uses React 19.2. |
| AsyncStorage | 3.1.1 | The [Expo SDK 57 bundle](https://github.com/expo/expo/blob/sdk-57/packages/expo/bundledNativeModules.json) uses 2.2.0 for Expo Go. Version 3 uses a new native module. |
| Gesture Handler | 3.3.0 | Expo SDK 57 uses 2.32.0 with its navigation packages. |
| Reanimated / Worklets | 4.7.1 / 0.13.0 | Expo SDK 57 uses 4.5.1 / 0.10.1. Screens 4.26.2 and Safe Area Context 5.7.0 stay on the same SDK baseline. |
| Java | Temurin 27 | Expo SDK 57 generates Gradle 9.3.1. [Java 27 requires Gradle 9.8](https://docs.gradle.org/current/userguide/compatibility.html), so Android builds use Temurin 25.0.4.1+1. Gradle supports Java 25 since 9.1.0. |
| PostgreSQL | 18.6 | PostgreSQL stays on 18.x. Version 19 is still in beta. PostHog's separate database stays on 14.1. |
| Keycloak | 26.8.0 | This is still the latest stable release. The theme matrix covers 26.7.5 and 26.8.0. |

Platform chart updates stay within their current major versions. The Tempo
chart moves to 2.4.0 and kube-prometheus-stack to 88.6.5. Tempo chart 3.1.0 and
kube-prometheus-stack 91.9.0 need separate chart migration checks. PostHog stays
on its last published chart, 30.46.0, and ClickHouse 22.8.21.38. Its Redis image
moves to the latest 6.2 patch, 6.2.24.

## Update rules

- The matrix changes only through a release-train PR in the baukit repository that runs the full test suite (unit, conformance, generated-fixture matrix) against the new versions.
- Renovate proposes baseline updates into baukit. Products pin Baukit crates exactly and can update third-party Rust dependencies within the declared caret ranges using their own lockfiles. Dependencies outside those ranges need a Baukit compatibility update.
- The upgrade-sensitive sets (OpenTelemetry crates; Expo/React/React Native) are always updated as grouped PRs.
