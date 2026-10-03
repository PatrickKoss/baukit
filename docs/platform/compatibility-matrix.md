# Dependency compatibility matrix

**Status:** Adopted.
**Home:** this repository; updated by the release train, not by hand-edits in products.

This table records what the shared baseline is **tested against**. Renovate keeps individual products moving; baukit guarantees compatibility only with the versions listed here. Version cells reflect the review-time state of the three projects and must be re-verified against the lockfiles when the baukit repository is created.

Last verified release train: `v0.6.0` (dependency refresh: utoipa 6,
OpenTelemetry 0.33 with tracing-opentelemetry 0.34, reqwest 0.13, Debian 13
backend images, ESLint 10, zod 4, Vitest 5, pnpm 12.7.0, the current Expo SDK 57
patches, and the current GitHub Actions majors; backend, web, mobile, combined,
and MCP generated fixtures with coverage floors, browser Dexie conformance with
36 tests, Docker-backed integration tests with 722 passing, both media-grant
vector suites, Expo SQLite conformance with 33 tests, Hermes vectors and
expo-notifications on Android, the generated Android compile, the MSRV check,
cargo deny, and the complete local CI-equivalent gates were verified locally
because hosted runners were unavailable). The iOS simulator gate requires macOS
and remains a release-host check rather than a Linux result.

## Toolchain

| Tool | Tested baseline | Notes |
|---|---|---|
| Rust | 1.95.0 MSRV | CI-enforced with Rust 1.95; train cut on stable 1.97.1 |
| Node | 24 (v24.20.0 test host) | pinned via `mise.toml`, `typescript/.nvmrc`, and package engines |
| Java | Temurin 21 (21.0.12.1 test host) | pinned via `mise.toml`; used by Android builds |
| Swift | 6.3.3 | pinned via `mise.toml`; compiler version checked on Linux |
| xtool | 1.17.0 | pinned via `mise.toml`; version checked on Linux, without a Darwin SDK or simulator |
| pnpm | 12.7.0 | pinned via `packageManager`; 12.8.0 to 12.8.2 reject fresh lockfiles under `--frozen-lockfile` when a `file:` dependency has an optional peer the importer provides |
| Turbo | 2 | |

## Backend (Rust)

| Responsibility | Dependency | Tested baseline | Notes |
|---|---|---|---|
| Async runtime | Tokio | 1.x (latest at train cut) | |
| HTTP | Axum | 0.8 | Tower / Tower HTTP at Axum-compatible versions |
| Persistence | SQLx | 0.9 | PostgreSQL 18.6, `runtime-tokio`, rustls |
| Database | PostgreSQL | 18.6 | Alpine in tests, generated Compose and PostHog; CNPG uses `18.6-system-trixie` for primary and restore clusters. |
| API description | Utoipa | 6 | |
| Traces | OpenTelemetry + tracing-opentelemetry | 0.33 + 0.34 | upgrade only as a matched set |
| Metrics | metrics + metrics-exporter-prometheus | latest compatible | one recorder per process |
| Logging | tracing + tracing-subscriber | latest compatible | |
| Configuration | config + dotenvy | chosen loader (analysis §4.1) | Figment is not supported by the shared kit |
| Outbound HTTP | reqwest | 0.13, rustls with the platform verifier | |
| Auth | ring + JWKS | latest | Keycloak OIDC default; Clerk session-token adapter with `azp` validation; WorkOS AuthKit adapter bound to `client_id`. `ApiTokenStore` returns `ApiTokenStoreError` since 0.3.0. |
| Development identity provider | Keycloak | 26.8.0 | Generated `compose.yaml` image; `make dev` reconciles the development realm from `realm-policy.json`. |
| Integration tests | testcontainers | latest | `baukit-test` pins `postgres:18.6-alpine`; templates and smoke deploys use the same image |
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
| Mobile runtime | Expo SDK | 57.0.26 (RN 0.86.3, React 19.2.8) | React/RN versions follow the Expo SDK, verified with Expo Doctor |
| Mobile navigation | Expo Router | 57.0.24 | Generated mobile template baseline with Expo Router and `Stack.Protected` in the auth overlay; `react-native-screens` 4.26.2, `react-native-safe-area-context` 5.7.0, `react-native-reanimated` 4.5.1, `react-native-worklets` 0.10.1, and `react-native-gesture-handler` 2.32.0. |
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
| MCP server | `@modelcontextprotocol/sdk` + zod | 1.31.0 + 4.6.5 | Opt-in `--mcp` generated stdio package; bearer tokens from `@baukit/auth-node` or a caller-supplied provider. |
| Web build | Vite | 8.3.2 | |
| Styling | Tailwind CSS | 4 | |
| Web persistence | `@baukit/data-contracts-dexie` / Dexie | 4.4.6 | only when offline is enabled; Chromium and WebKit conformance-tested |
| Native scoped persistence digest | `expo-crypto` | 57.0.3 | Expo adapter injected into the identity-scoping contract |
| Native accessibility lint | `eslint-plugin-react-native-a11y` + `@eslint/compat` | 3.5.1 + 2.1.1 | Generated mobile template lint baseline on ESLint 10; the plugin declares an ESLint 8 peer, so the template allows ESLint 10 through `peerDependencyRules` |
| Web accessibility checks | axe-core | 4.13.0 | Serious/critical jsdom scan seam; contrast remains a real-browser check |
| Web e2e | Playwright | 1.63.0 | Chromium 153.0.8010.12 (revision 1243) and WebKit 26.6 (revision 2359) |
| Android native compile | Expo prebuild + Gradle | API 36, build-tools 36.0.0, Java 21 | Blocking for relevant generated-product and Baukit fixture changes |
| Native e2e | Maestro | latest | Configurable for product-owned critical paths; scheduled/manual, not part of the universal pull-request promise |
| iOS native compile | Xcode + iOS Simulator | macOS runner | Scheduled/manual; Linux is recorded as blocked, never as a passing skip |

## Update rules

- The matrix changes only through a release-train PR in the baukit repository that runs the full test suite (unit, conformance, generated-fixture matrix) against the new versions.
- Renovate proposes updates into baukit; products receive them by upgrading their baukit version, not by diverging individually.
- The upgrade-sensitive sets (OpenTelemetry crates; Expo/React/React Native) are always updated as grouped PRs.
