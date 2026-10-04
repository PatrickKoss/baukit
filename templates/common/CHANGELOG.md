# Changelog

## [Unreleased]

{% if context.mobile %}- Match React and the react-dom override to Expo 57.0.26 bundled version 19.2.3. Keep React in Expo dependency checks.
{% endif %}

- Install Corepack 0.36.0 before using pnpm in CI. Node 26 does not bundle Corepack.
- Update pnpm to 12.9.1. Fresh web, mobile, and MCP lockfiles now pass frozen installation with linked Baukit packages.
{% if context.mobile %}- Match Jest types to the Jest 29 runtime.
{% endif %}{% if context.web or context.mobile or context.mcp %}- Update ESLint to 10.12 and Node types to 26.6.4.
{% endif %}{% if context.web %}- Update TanStack Query to 5.104.1.
{% endif %}{% if context.mcp %}- Update MCP SDK to 1.32.
{% endif %}{% if context.backend %}- Update Tokio to 1.53.2 and UUID to 1.27.
{% endif %}
{% if context.backend %}- Pin the development database to PostgreSQL `18.6-alpine`. Mount its volume
  at `/var/lib/postgresql` and recreate local volumes on major-version changes.
{% endif %}- Generated `AGENTS.md` is a regular file, with the same guidance as `CLAUDE.md`, for Turbo workspace discovery.
{% if context.mobile %}- Aligned both mobile templates on `expo-constants` 57.0.20. Pinned `react-dom` to 19.2.8 alongside mobile React and `@react-native/metro-config` to React Native 0.86.3, allowed the ESLint 10 peer for the accessibility plugin, and disabled the `core-js` postinstall through pnpm 12 `allowBuilds`.
{% endif %}{% if context.backend and context.auth_oidc %}- Moved the development Keycloak image to 26.8.0. The theme browser matrix covers 26.7.5 and 26.8.0.
- Moved the development Redis image to 8.10.2 Alpine.
{% endif %}{% if context.backend %}- Changed generated API DTOs to camelCase JSON names and the error envelope to `requestId`. `backend/tests/openapi_drift.rs` now also fails on any property or path or query parameter name that is not camelCase, and the strict quality gate runs it.
- Added error-response rules to the generated OpenAPI document. Every operation now documents the 400, {% if context.auth_oidc %}401, {% endif %}404, 413, 415, 422, {% if context.auth_oidc %}429, {% endif %}500, and 504 responses its middleware can return, with `X-Request-Id` on every response.
- Changed the backend to utoipa 6, which the Baukit OpenAPI helpers now require. Code that builds utoipa values by hand must wrap operation parameters and response headers in `RefOr::T`, and `HeaderBuilder::schema` takes an `Option`.
- Changed `backend/Dockerfile` to build on `rust:1.99.0-trixie` and run on `gcr.io/distroless/cc-debian13:nonroot`. Builder and runtime moved to Debian 13 together, so the binaries and the runtime share one glibc.
{% if context.quality_strict %}- Added `quality.openapi_compatibility` (`off`, `report`, or `enforce`) to the strict quality gate. It compares `backend/openapi.json` with the base revision and reads accepted breaks from `docs/openapi-accepted-breaks.json`.
{% endif %}{% endif %}{% if context.mcp %}- Added `dateTimeInput` for MCP timestamp fields. It accepts zoned timestamps with or without seconds and rejects invalid dates and missing timezones.
- Added the opt-in MCP stdio package with explicit tool registries, bearer-token providers, and OpenAPI route checks.
{% endif %}{% if context.mobile %}- Added mobile tests for the theme mode control, record store seams, and the route heading focus hook, with `@testing-library/react-native` and `test-renderer` as dev dependencies.
{% if context.auth_oidc %}- Changed the auth mobile app to use the base Jest config, so `test:coverage` now enforces the 70% statement, branch, function, and line floors. The app also gains the `setup` and `test:coverage` scripts that the generated CI already calls. New tests cover the OIDC auth hook, the authenticated local-data provider, and the authenticated API runtime.
{% endif %}{% endif %}{% if context.web and context.auth_oidc %}- Added `E2E_WEB_PORT` to the web Keycloak stack test. Its global setup adds the chosen origin to the realm's web client through the admin API when the client lacks it, and `E2E_KEYCLOAK_WEB_CLIENT_ID` names that client.
- Changed the web Keycloak stack test to import `keycloakStack`, `createKeycloakTestUser`, `signInWithKeycloak`, and `allowKeycloakWebOrigin` from `@baukit/auth-node/keycloak-testing`, now a web dev dependency. `e2e/stack/keycloak.ts` keeps only this product's defaults and exports them as `stack`; the helpers take `stack` explicitly and no longer take a Playwright request context.
- Added auth web tests for the OIDC client wiring, the API parsers and default transport, the authenticated API runtime, and the local-data hook, so `test:coverage` passes its 70% floors.
{% endif %}{% if context.backend and context.auth_oidc %}- Added a Redis service to `compose.yaml` on `127.0.0.1:{{ context.redis_host_port }}`. The API's rate limiter needs it at startup, and `make dev` now starts it.
{% endif %}{% if context.mobile %}- Moved the native workflow to `gradle/actions/setup-gradle@v6` with `cache-provider: basic`, which keeps the MIT-licensed cache instead of v6's default proprietary one.
{% endif %}{% if context.web or context.mobile %}- Moved linting to ESLint 10, `@eslint/js` 10, and `typescript-eslint` 8.71. TypeScript stays on 6.0 because `typescript-eslint` does not accept TypeScript 7 yet.
{% endif %}{% if context.web or context.mobile or context.mcp %}- Changed `eslint.config.js` to `defineConfig` from `eslint/config`, since `tseslint.config` is deprecated, and pinned `@types/node` to 24.19 to match the Node 24 runtime.
{% endif %}{% if context.web %}- Moved the web app to React 19.3, Vite 8.3, Vitest 5, `@vitejs/plugin-react` 6.1, Playwright 1.63, and jsdom 30.1.
{% endif %}{% if context.mobile %}- Moved the mobile app to the current Expo SDK 57 patch releases (`expo` 57.0.26, `react-native` 0.86.3, `expo-router` 57.0.24, `jest-expo` 57.0.5), i18next 26.4, and react-i18next 17.0. `eslint-plugin-react-native-a11y` still declares ESLint 8 as its peer, so `pnpm-workspace.yaml` allows ESLint 10 for it and `@eslint/compat` wraps its rules. `test-renderer` stays on 1.2.0 because 1.3.0 needs React 19.3, which React Native 0.86 does not support.
{% endif %}{% if context.mcp %}- Moved the MCP package from TypeScript 5.9 to 6.0 and to Vitest 5 and zod 4.6.
{% endif %}- Added append-only `.env` reconciliation to generated project setup. Existing local bytes and values are preserved.
- Fixed the strict quality gate so a freshly generated project can run it before its first commit.
- Added a dependency-free local Markdown link check to the strict quality profile.
- Changed the pinned pnpm to 12.7.0 in `packageManager`, the scripts, and CI. pnpm 12.8.0 through 12.8.2 reject a fresh lockfile under `--frozen-lockfile` when a `file:` dependency has an optional peer that the importer provides.
- Moved the generated GitHub workflows to `actions/checkout@v7`, `actions/cache@v6`, `actions/setup-node@v7`, `actions/setup-java@v6`, `actions/upload-artifact@v7`, and `dorny/paths-filter@v4`.
