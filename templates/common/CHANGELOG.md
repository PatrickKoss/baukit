# Changelog

## [Unreleased]

- Place shipped authentication template notes under their release headings.

- Share configuration collection declarations across the API, migration and worker commands. Parse MCP hosts and origins as JSON arrays in every command.

{% if context.auth_oidc %}- Reconcile Keycloak client scopes and their bindings. Repair scope protocol mappers through their Admin API endpoints.

{% endif %}{% if context.mcp or context.mobile %}{% if context.mcp %}- Breaking: MCP tool adapters receive a request cancellation token and stop pending item reads when cancelled.
- Set the product name, version and instructions in MCP server discovery. Keep the default annotations explicit in tool declarations.
{% endif %}{% if context.mobile %}- Breaking: Native checks now require Java 25. Use Android command-line tools 23.0 in QA. Select the macOS download for the host CPU.
- Grant native access to Android's Prefab tool during native builds. Preserve existing JVM options.
{% endif %}
{% endif %}{% if context.mcp %}- Keep product-crate imports in their own group in the MCP crate and drift test, so rustfmt output no longer depends on the product name.

{% endif %}## [0.9.0] - 2026-10-07

{% if context.mcp %}- Fetch signing keys over the internal network while keeping the public issuer. Add an optional backend Compose profile.
- Register MCP services through `McpServices`.
- Check tools, resources and prompts in MCP definition drift tests. Accept default tool errors in the example output schema.
- Fix rustfmt drift in the MCP router wiring for short product names.

{% endif %}{% if context.web %}- Measure the screen and scroller together in the browser geometry check, so navigation animation cannot mix two layouts.

{% endif %}{% if context.mobile %}- Control the animation frame in the Android navigation test so dismissal checks do not depend on elapsed time.
- Keep Android QA ownership until cleanup succeeds. Verify emulator shutdown and report service teardown failures.
- Reject System UI and keyboard crashes or ANRs after boot and before Maestro flows.
- Limit the Android software renderer to four threads by default. Set `BAUKIT_QA_RENDERER_THREADS` to change the limit.

{% endif %}{% if context.auth_enabled %}- Generate {{ context.auth_provider }} sign-in, token verification and profile deletion. MCP applies the provider's OAuth token binding.

{% endif %}## [0.8.0] - 2026-10-07

{% if context.mcp %}- Breaking: `--mcp` now generates the Rust remote server and requires `--backend --auth oidc`. The TypeScript stdio template and its authentication flags are removed. Set `capabilities.mcp = true` after migrating existing tools.
{% endif %}{% if context.mobile %}- Update Expo SDK 57 patches to match Expo compatibility checks.
{% endif %}

{% if context.mcp %}- Add an opt-in Rust remote MCP template with Keycloak PKCE, deployment routes, exact Host and Origin configuration, and scoped service ports.

{% endif %}## [0.7.4] - 2026-10-06

{% if context.mobile and context.pwa and not context.web %}- Generate Expo web dependencies, SQLite WASM assets and single-page output for the mobile PWA host. Build and check the worker in the exported site.
{% endif %}{% if context.mobile %}- Set the Android Gradle dev-server address to loopback and give QA emulators four CPU cores.
- Apply the top safe-area inset to tab scenes and the sign-in header.
- Build the iOS simulator app in Release mode so Maestro has an embedded JavaScript bundle.
- Generate service worker build and check scripts for Expo web.
{% endif %}{% if context.mobile and context.auth_oidc %}- Store OIDC sessions in browser storage on web and SecureStore on native platforms.
{% endif %}{% if context.auth_oidc %}- Test expired access tokens separately from valid refresh sessions without waiting for expiry.
{% endif %}
- Keep the 0.7.2 changes under their release heading.

## [0.7.3] - 2026-10-05

{% if context.auth_oidc %}- Reconcile confidential backend clients without browser URLs. Preserve creation and rotated secrets.
- Retain failed identity deletion jobs for repair when cleaning terminal jobs.
{% endif %}{% if context.mobile %}- Reserve compact navigation height for the font scale and safe-area bottom inset.
{% endif %}{% if context.web or context.mobile %}- Document 48 dp Android navigation targets alongside the web and iOS minimums.
{% endif %}{% if context.quality_strict %}- Accept Markdown links to paths with balanced parentheses, including Expo route groups, and angle-bracket targets.
{% endif %}{% if context.mobile %}- Stop Android QA setup when the device probe fails or times out.
{% endif %}{% if context.worker %}- Use `job_kind` to query worker metrics separately from the Prometheus scrape job.
{% endif %}{% if context.mobile and context.auth_oidc %}- Use SecureStore-safe OIDC keys without a product key rewrite.
{% endif %}{% if context.mobile %}- Add expo-system-ui 57.0.4 to support automatic mobile appearance with Expo 57.
{% endif %}{% if context.web %}- Store Playwright browsers in web/.playwright-browsers so dependency reinstalls preserve them. Exclude the cache from Docker contexts.
- Bind the web preview server to 127.0.0.1 so Playwright readiness uses the same address.
{% endif %}
## [0.7.2] - 2026-10-05

{% if context.quality_strict and context.backend %}
- Create the LCOV output directory when Cargo uses an external target directory.
{% endif %}{% if context.mobile %}
- Pin Android command-line tools to build 13114758. Recreate an AVD when its API level, image tag, or architecture changes.
- Bound ADB startup and shutdown probes and the emulator boot wait.
- Remove Expo dependency check exclusions. Keep Jest and its types on major 29.
- Scroll native smoke flows to the consent button before tapping it.
{% if context.auth_oidc %}- Use a valid SecureStore key for the local-data registry so authenticated local data can open.
{% endif %}{% endif %}{% if context.web or context.mobile %}
- Pass navigation labels from English and German catalogs.
{% if context.mobile %}- Prepare Chrome before OIDC QA flows so its first-run screen does not block sign-in.
- Replace Android emulator config keys with spaces around `=` without leaving duplicate entries.
{% if context.auth_oidc %}- Return mobile OIDC callbacks to the app root after sign-in.
{% endif %}{% endif %}{% endif %}
## [0.7.1] - 2026-10-04

{% if context.web %}- Center generated web content beside the navigation rail. Keep action links at least 44 pixels high.
- Check initial dialog focus by accessible name. Support labeled inputs and buttons named by visible text.

{% endif %}{% if context.mobile %}- Match React and the react-dom override to Expo 57.0.26 bundled version 19.2.3. Keep React in Expo dependency checks.
{% endif %}

## [0.7.0] - 2026-10-04

- Install Corepack 0.36.0 before using pnpm in CI. Node 26 does not bundle Corepack.
- Update pnpm to 12.9.1. Fresh web and mobile lockfiles now pass frozen installation with linked Baukit packages.
{% if context.mobile %}- Match Jest types to the Jest 29 runtime.
{% endif %}{% if context.web or context.mobile %}- Update ESLint to 10.12 and Node types to 26.6.4.
{% endif %}{% if context.web %}- Update TanStack Query to 5.104.1.
{% endif %}{% if context.backend %}- Update Tokio to 1.53.2 and UUID to 1.27.
{% endif %}
{% if context.backend %}- Pin the development database to PostgreSQL `18.6-alpine`. Mount its volume
  at `/var/lib/postgresql` and recreate local volumes on major-version changes.
{% endif %}- Generated `AGENTS.md` is a regular file, with the same guidance as `CLAUDE.md`, for Turbo workspace discovery.
{% if context.mobile %}- Aligned both mobile templates on `expo-constants` 57.0.20. Pinned `react-dom` to 19.2.8 alongside mobile React and `@react-native/metro-config` to React Native 0.86.3, allowed the ESLint 10 peer for the accessibility plugin, and disabled the `core-js` postinstall through pnpm 12 `allowBuilds`.
{% endif %}{% if context.backend and context.auth_oidc %}- Moved the development Keycloak image to 26.8.0. The theme browser matrix covers 26.7.5 and 26.8.0.
- Moved the development Redis image to 8.10.2 Alpine.
{% endif %}

## [0.6.0] - 2026-10-02

{% if context.backend %}- Changed generated API DTOs to camelCase JSON names and the error envelope to `requestId`. `backend/tests/openapi_drift.rs` now also fails on any property or path or query parameter name that is not camelCase, and the strict quality gate runs it.
- Added error-response rules to the generated OpenAPI document. Every operation now documents the 400, {% if context.auth_oidc %}401, {% endif %}404, 413, 415, 422, {% if context.auth_oidc %}429, {% endif %}500, and 504 responses its middleware can return, with `X-Request-Id` on every response.
- Changed the backend to utoipa 6, which the Baukit OpenAPI helpers now require. Code that builds utoipa values by hand must wrap operation parameters and response headers in `RefOr::T`, and `HeaderBuilder::schema` takes an `Option`.
- Changed `backend/Dockerfile` to build on `rust:1.99.0-trixie` and run on `gcr.io/distroless/cc-debian13:nonroot`. Builder and runtime moved to Debian 13 together, so the binaries and the runtime share one glibc.
{% if context.quality_strict %}- Added `quality.openapi_compatibility` (`off`, `report`, or `enforce`) to the strict quality gate. It compares `backend/openapi.json` with the base revision and reads accepted breaks from `docs/openapi-accepted-breaks.json`.
{% endif %}{% endif %}{% if context.mobile %}- Added mobile tests for the theme mode control, record store seams, and the route heading focus hook, with `@testing-library/react-native` and `test-renderer` as dev dependencies.
{% if context.auth_oidc %}- Changed the auth mobile app to use the base Jest config, so `test:coverage` now enforces the 70% statement, branch, function, and line floors. The app also gains the `setup` and `test:coverage` scripts that the generated CI already calls. New tests cover the OIDC auth hook, the authenticated local-data provider, and the authenticated API runtime.
{% endif %}{% endif %}{% if context.web and context.auth_oidc %}- Added `E2E_WEB_PORT` to the web Keycloak stack test. Its global setup adds the chosen origin to the realm's web client through the admin API when the client lacks it, and `E2E_KEYCLOAK_WEB_CLIENT_ID` names that client.
- Changed the web Keycloak stack test to import `keycloakStack`, `createKeycloakTestUser`, `signInWithKeycloak`, and `allowKeycloakWebOrigin` from `@baukit/auth-node/keycloak-testing`, now a web dev dependency. `e2e/stack/keycloak.ts` keeps only this product's defaults and exports them as `stack`; the helpers take `stack` explicitly and no longer take a Playwright request context.
- Added auth web tests for the OIDC client wiring, the API parsers and default transport, the authenticated API runtime, and the local-data hook, so `test:coverage` passes its 70% floors.
{% endif %}{% if context.backend and context.auth_oidc %}- Added a Redis service to `compose.yaml` on `127.0.0.1:{{ context.redis_host_port }}`. The API's rate limiter needs it at startup, and `make dev` now starts it.
{% endif %}{% if context.mobile %}- Moved the native workflow to `gradle/actions/setup-gradle@v6` with `cache-provider: basic`, which keeps the MIT-licensed cache instead of v6's default proprietary one.
{% endif %}{% if context.web or context.mobile %}- Moved linting to ESLint 10, `@eslint/js` 10, and `typescript-eslint` 8.71. TypeScript stays on 6.0 because `typescript-eslint` does not accept TypeScript 7 yet.
{% endif %}{% if context.web or context.mobile %}- Changed `eslint.config.js` to `defineConfig` from `eslint/config`, since `tseslint.config` is deprecated, and pinned `@types/node` to 24.19 to match the Node 24 runtime.
{% endif %}{% if context.web %}- Moved the web app to React 19.3, Vite 8.3, Vitest 5, `@vitejs/plugin-react` 6.1, Playwright 1.63, and jsdom 30.1.
{% endif %}{% if context.mobile %}- Moved the mobile app to the current Expo SDK 57 patch releases (`expo` 57.0.26, `react-native` 0.86.3, `expo-router` 57.0.24, `jest-expo` 57.0.5), i18next 26.4, and react-i18next 17.0. `eslint-plugin-react-native-a11y` still declares ESLint 8 as its peer, so `pnpm-workspace.yaml` allows ESLint 10 for it and `@eslint/compat` wraps its rules. `test-renderer` stays on 1.2.0 because 1.3.0 needs React 19.3, which React Native 0.86 does not support.
{% endif %}- Added append-only `.env` reconciliation to generated project setup. Existing local bytes and values are preserved.
- Fixed the strict quality gate so a freshly generated project can run it before its first commit.
- Added a dependency-free local Markdown link check to the strict quality profile.
- Moved the generated GitHub workflows to `actions/checkout@v7`, `actions/cache@v6`, `actions/setup-node@v7`, `actions/setup-java@v6`, `actions/upload-artifact@v7`, and `dorny/paths-filter@v4`.
