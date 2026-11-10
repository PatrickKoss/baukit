# Dependency compatibility matrix

**Status:** Adopted.
**Home:** this repository; updated by the release train, not by hand-edits in products.

This table records the tested baseline. The committed lockfiles record exact resolutions. Third-party Rust manifests use caret requirements with tested minimums, so products can take compatible updates in their own lockfiles without a Baukit release. A requirement change must pass tests at the direct minimums and the newest compatible resolution. Internal `baukit-*` requirements stay exact because the crates release together.

## v0.21.0

Last verified release train: `v0.21.0` (minor train on top of `v0.20.0`, breaking for products that build SQL at runtime).
- `baukit check` rejects `AssertSqlSafe` in production Rust, `format!` SQL that captures an identifier such as `{COLUMNS}`, and `replace`/`replacen` on SQL loaded with `include_str!`. Test code is exempt.
- `baukit-auth` compares the OIDC discovery `issuer` with the configured issuer string, so an issuer without a trailing slash no longer fails with `IssuerMismatch`.
- Templates: `scripts/process_runner.py` adds `start_process`, the single spawn point for long-running children. `run_process` now runs on it.

### Upgrade steps

1. Set every `baukit-*` crate and `@baukit/*` package to `0.21.0`, and set `baukit.toml`'s `template_version` and `[dependencies.baukit].version` to `0.21.0`. Strict products move the Baukit clone in `scripts/quality-gate.sh` to `--branch v0.21.0`. For the first day, list the exact `@baukit/<name>@0.21.0` entries in `minimumReleaseAgeExclude`.
2. Copy `scripts/process_runner.py` and replace direct `subprocess.Popen` calls with `start_process`.
3. Replace runtime-built SQL with one `&'static str` per query variant and bound parameters (optional filters as `($n IS NULL OR ...)`, lists as `= ANY($n)`), or a `QueryBuilder` with `push_bind` for truly dynamic shapes. Keep the owner in the `WHERE`.
4. Drop fixture workarounds that served the issuer with a trailing slash.

## v0.20.0

Last verified release train: `v0.20.0` (minor train on top of `v0.19.2`, breaking for `baukit-suite` link stores and `baukit-egress` status errors).
- `baukit-suite` selects suite links by owner in SQL. `SuiteLinkStore::link_for_update`, `revoke_in_transaction`, `record_delivery`, `complete_delivery`, `record_received_in_transaction` and `replay_retry_after` take the owner.
- `baukit-egress` keeps allowlisted response headers on `EgressError::Status` in a new `headers` field.
- `baukit-telemetry` caps `rmcp` at `info` in the default log filter.
- `baukit-auth` `BrowserSessionConfig::with_internal_issuer` lets a containerized backend reach Keycloak without moving the public issuer.
- `@baukit/auth-web` treats only the configured redirect URI as an OIDC callback and takes login options for appearance state and a registration entry. `@baukit/csp` adds vendor script and frame sources, `blob:` frames and a production origin guard. `@baukit/a11y-core` attempts route focus before checking the retry deadline.
- `baukit check` flags SQL fragments, SQL splicing, lying type predicates and response casts, skips Cargo manifests that do not parse, and `template-files --update` rewrites the hashes. osv scans only the packages cargo can build.
- Templates: no global ruff ignores, subprocesses through `scripts/process_runner.py`, QA extension hooks, a product token source, a secret-free Keycloak `smtpServer`, erase and purge of browser sessions, `localhost` for every local web origin, read-only fuzz seed corpora, and two parallel fixture lanes in the baukit gate.

### Upgrade steps

1. Set every `baukit-*` crate and `@baukit/*` package to `0.20.0`, and set `baukit.toml`'s `template_version` and `[dependencies.baukit].version` to `0.20.0`. Strict products move the Baukit clone in `scripts/quality-gate.sh` to `--branch v0.20.0`. For the first day, list the exact `@baukit/<name>@0.20.0` entries in `minimumReleaseAgeExclude`.
2. Apply the `[0.20.0]` sections of the template changelogs. Copy `scripts/process_runner.py`, drop the global ruff ignores (`E501`, `S603`, `S607`, `PLR2004`, `PLR0913`) and fix the findings, and move product divergences in `.baukit/template-files.toml` into the new hooks.
3. Pass the owner to the changed `SuiteLinkStore` methods, and add `headers` (or `..`) where code builds or fully destructures `EgressError::Status`.
4. Run `baukit check --root .` and fix what the new patterns find with guards and bound parameters, not suppressions. Drop product copies of the `rmcp` log cap and the CSP origins that `@baukit/csp` now provides.

## v0.19.2

Last verified release train: `v0.19.2` (patch train on top of `v0.19.1`).
- The mobile web build no longer stores tokens in browser storage. On web, `auth-storage.ts` throws `WebTokenStorageRefused`, and Expo web signs in through `@baukit/auth-web`.
- A backend whose only browser client is the PWA now serves the `/auth/session` browser-session routes. The new `mobile-pwa-auth` fixture flavor covers it.
- `baukit-auth` normalizes browser-session redirect URIs.
- `baukit-egress` reads vendor rate-limit headers and trusts extra root certificates.
- `baukit check suppressions` rejects bulk suppression and lint config weakening, and budgets seven more comment forms.
- `rmcp` is 3.9.0, past RUSTSEC-2026-0457, RUSTSEC-2026-0461 and RUSTSEC-2026-0462.

### Upgrade steps

1. Set every `baukit-*` crate and `@baukit/*` package to `0.19.2`, and set `baukit.toml`'s `template_version` and `[dependencies.baukit].version` to `0.19.2`. Strict products move the Baukit clone in `scripts/quality-gate.sh` to `--branch v0.19.2`. For the first day, list the exact `@baukit/<name>@0.19.2` entries in `minimumReleaseAgeExclude`, as in 0.19.1.
2. Add the new `[suppressions.max]` keys to `baukit-check.toml`: `ts-expect-error`, `ts-ignore`, `prettier-ignore`, `noqa`, `type-ignore`, `nosec`, `shellcheck-disable` and `nolint`. Each starts at the product's current count, and the counts only go down. Give every remaining directive the reason form `baukit check suppressions` asks for.
3. Delete any `eslint-suppressions.json` and any `--suppress-*` flag, and fix the findings they hid. Remove every `"allow"` level from `[lints]` tables, every clippy `-A`/`--allow`/`--cap-lints` flag in gate scripts, and every eslint rule a product config sets to `off` or `warn`. Fix the code that each of these hid.
4. Mobile products with `pwa` and auth apply the mobile template changelog entry for 0.19.2. Then register the web origin as a redirect URI, and the backend serves the browser-session routes, as described in the template changelog.
5. Products that call vendors with their own rate-limit headers (Fitbit, Whoop, Polar) switch to `EgressOptions::with_retry_after_headers`. Products that pin a private CA switch to `with_extra_root_certificates_pem`.
6. Products with an MCP server run `cargo update -p rmcp` to reach 3.9.0. Backend products add the RUSTSEC-2026-0459 `[[IgnoredVulns]]` entry from the new `osv-scanner.toml` (`ignoreUntil = 2027-02-07`), because sqlx 0.9.0 caps `libsqlite3-sys` below the fixed 0.39 and no crate compiles it.

## v0.19.1

Last verified release train: `v0.19.1` (patch train on top of `v0.19.0`). The `v0.19.0` tag run died in its verify step, so its crates never reached crates.io; its npm packages were published by hand. `v0.19.1` is the first train the release workflow publishes to both registries through trusted publishing. The strict template's fuzz step passes the nightly host triple to `cargo fuzz`, because the prebuilt cargo-fuzz is a musl binary and ASan rejects musl targets. The release workflow verifies the TypeScript train with the same `ts-check` gate step as CI.

### Upgrade steps

A product on 0.18.x goes straight to 0.19.1: follow every step of the v0.19.0 section below with `0.19.1` in place of `0.19.0`, then apply these.

1. Set every `baukit-*` crate and `@baukit/*` package to `0.19.1` and `baukit.toml`'s `template_version` and `[dependencies.baukit].version` to `0.19.1`. Strict products move the Baukit clone in `scripts/quality-gate.sh` to `--branch v0.19.1`.
2. Strict products copy the fuzz step from a fresh `baukit new` render (`--target "$host"` on `cargo +nightly fuzz build` and `run`).
3. pnpm refuses `@baukit/*@0.19.1` for a day after the publish. A product that installs inside that window lists the exact `@baukit/<name>@0.19.1` entries it needs in `minimumReleaseAgeExclude` and removes them once the window ends.

## v0.19.0

Last verified release train: `v0.19.0` (minor train on top of `v0.18.3`), the quality and security train. The full local gate passed on 2026-11-07 with Rust 1.95 MSRV, Node 26.10.0 and pnpm 12.9.1: every Rust test with `--include-ignored`, all generated fixture flavors (backend, web, mobile, mobile-pwa, combined, mcp, auth, clerk, workos, platform-standalone, platform), the remote MCP fixture against Keycloak, the real-browser Dexie suite and both Android conformance runs. Three npm packages are new: `@baukit/type-guards`, `@baukit/eslint-config` and `@baukit/csp`.

### Breaking changes

- `baukit-sync`: `TombstoneTable.delete` binds ids as `$1` and owners as `$2` and must delete `(id, owner_id)` pairs. The old one-parameter SQL deleted every owner's row with a purged id.
- `baukit-suite`: `adapters::http::router` is gone. Mount the suite with `PostgresSuite::router(api)` or `adapters::public_limit::limited_router`, which rate limit callback, exchange and revoke per client IP.
- `baukit-auth`: `ClerkVerifier` rejects tokens without `azp` unless `allow_missing_authorized_party()` is set; `IssuedApiToken` lost `PartialEq`; `VerificationError` carries egress errors, because `OidcVerifier` fetches discovery and JWKS through `baukit-egress`.
- `baukit-http`: `layers()` adds security headers to every response, including `Content-Security-Policy: default-src 'none'; frame-ancestors 'none'`. A backend route that serves HTML sets its own CSP.
- `@baukit/auth-web`: tokens leave the browser. `BrowserSessionClient` redeems the code through the backend's `/auth/session/{exchange,refresh,logout}` (ADR 0006), and `OidcClient` keeps only discovery, login and the callback transaction. There is no compatibility mode.
- Templates: `AGENTS.md` is a symlink to `CLAUDE.md` with the managed agent-rules block, and the `items` example is owner-scoped.

### Upgrade steps

Follow the sections in this order, starting from a product on 0.18.2 or 0.18.3. Render a fresh product with the same capabilities (`baukit new`) at v0.19.0 to copy files from.

#### Bump the pins

1. Set every `baukit-*` crate to `=0.19.0` and every `@baukit/*` package to `0.19.0`, and set `baukit.toml`'s template version to `0.19.0`. Add `@baukit/eslint-config` and `@baukit/csp` as dev dependencies of web and mobile.
2. Strict products move the Baukit clone in `scripts/quality-gate.sh` to `git clone --branch v0.19.0`, and `baukit_commit` to that tag's commit when the gate pins one.
3. pnpm refuses `@baukit/*@0.19.0` for a day after the publish. Inside that window list the exact `@baukit/<name>@0.19.0` entries in `minimumReleaseAgeExclude` and remove them afterwards. Remove any `@baukit/*@0.18.x` entries left over.
4. Refresh `backend/Cargo.lock` and the pnpm lockfiles.

#### Owner-keyed tombstone purge (sync products)

Rewrite every `TombstoneTable.delete` to `DELETE FROM <table> AS record USING UNNEST($1::uuid[], $2::uuid[]) AS purged (id, owner_id) WHERE record.id = purged.id AND record.owner_id = purged.owner_id`. Add a test where two owners hold the same id and a purge for one leaves the other's row.

#### Suite routes (suite products)

Replace `adapters::http::router` with `PostgresSuite::router(api)` or `limited_router(state, store, limit)`. Every route, including `/suite/*` and `/platform/*` routes merged late, sits inside `baukit_ratelimit::layers` or its own limiter.

#### Provider linking (integration products)

Move OAuth provider-linking callbacks to `baukit_integrations::oauth_state` (features `oauth-state`, `postgres`, `axum`): a single-use 256-bit state bound to the browser with the `__Host-baukit-oauth` cookie, PKCE S256, and the `OAUTH_STATE_MIGRATION_SQL` migration. A signed, TTL-bound state that is neither single-use nor browser-bound lets an attacker link the victim's provider account.

#### Browser sessions (web products with auth)

Mount `baukit_auth::browser_session::router` (feature `browser-session`) with `StoreBrowserSessionRateLimiter` from `baukit-ratelimit`, and run its `0002_baukit_auth_browser_sessions.sql` migration. Copy the web template's sign-in files, which use `BrowserSessionClient`. Delete any code that reads `<prefix>:tokens`.

#### Clerk on mobile (Clerk products with an Expo app)

Native requests carry no `Origin`, so Clerk omits `azp`. Set `allow_missing_azp` in the product's Clerk config, which calls `ClerkVerifier::allow_missing_authorized_party()`. A present `azp` must still be allowed.

#### Web headers and CSP (web products)

Delete the local `csp/policy.ts` (and `src/csp/policy.ts` in mobile) and import the builder from `@baukit/csp`; keep only `origins.ts`. `vite preview` and the chart's `web` component send COOP and CORP `same-origin`. COEP stays unset. A product that opens a cross-origin popup and reads `window.opener` needs `same-origin-allow-popups` on its app pages and no COOP on the callback page. WebKit severs `window.opener` under any COOP, so the callback page also posts the result and its `state` on a `BroadcastChannel` (ADR 0006 section 7). Web app and API must use the same host name locally: `localhost` and `127.0.0.1` are different sites for cookies and CORS (ADR 0006 section 8).

#### Lint baseline, gate steps and agent docs

1. Copy the backend's workspace `[lints]`, `clippy.toml` and `deny.toml`, the web and mobile eslint configs on `@baukit/eslint-config`, `baukit-check.toml`, `.baukit/template-files.toml`, `.gitleaks.toml`, `zizmor.yml`, `osv-scanner.toml` and the `mise.toml` tools. Fix every finding; a `#[expect]` or `eslint-disable` names its invariant.
2. Copy the CI workflow and `scripts/repo-lint.sh`: cargo-deny, gitleaks, zizmor, osv-scanner, and `baukit check agent-docs file-size suppressions lints patterns observability template-files`. Product-specific gate steps go in `scripts/quality-gate.local.sh`.
3. Copy `backend/fuzz` and add a target for each parser or decoder the product owns.
4. Run `baukit check agent-docs --fix --root .` to refresh the managed block and turn `AGENTS.md` into the symlink.
5. Write `docs/threat-model.md` with the `baukit-threat-model` skill and test its controls with `baukit-security-testing`.

## v0.18.3

Last verified release train: `v0.18.3` (patch train on top of `v0.18.2`). The web template reads every `VITE_*` key statically, so Vite no longer inlines the whole env object, local issuer included, into a production bundle. `baukit-http` allows the `x-organization-id` request header in its default CORS settings, and `useOrganizationPlan` in `@baukit/purchases-core` returns a `retry()`. The mobile template's scripts pass `preserve-caught-error` and `@typescript-eslint/no-dynamic-delete`, its app-config tests run under Jest, and `--affected` runs the Android steps for a Maestro-only change. Baukit's workspaces and templates carry no `minimumReleaseAgeExclude` entries for `@baukit/*@0.18.2`, checked at 01:44 UTC on 2026-11-07, so the train removed none.

### Upgrade steps

Follow the sections in this order, starting from a product on 0.18.2. Render a fresh product with the same capabilities (`baukit new`) at v0.18.3 to copy files from.

#### Bump the pins

1. Set every `baukit-*` crate to `=0.18.3` and every `@baukit/*` package to `0.18.3`, and set `baukit.toml`'s template version to `0.18.3`.
2. Strict products move the Baukit clone in `scripts/quality-gate.sh` to `git clone --branch v0.18.3`. A product whose gate also pins `baukit_commit` sets it to the `v0.18.3` tag's commit in the same change. The `baukit-doctor` step builds the CLI from that pinned clone, so a stale tag runs an old doctor.
3. pnpm refuses `@baukit/*@0.18.3` for a day after the publish. A product that installs inside that window lists the exact `@baukit/<name>@0.18.3` entries it needs in `minimumReleaseAgeExclude` and removes them once the window ends. Remove any `@baukit/*@0.18.2` entries left from the last round.
4. Refresh `backend/Cargo.lock` with `cargo update -p baukit-core` (or any pinned crate) and the pnpm lockfiles with `corepack pnpm@12.9.1 install`.

#### Recorded platform responses

A product that keeps a recorded platform API responses fixture, such as a JSON file of hub `GET /platform/v1/organizations/{id}` bodies, adds the two required `OrganizationSummary` booleans `sso_connection_active` and `directory_sync_active` to every organization summary in it (AHP.ID). The 0.18.2 client rejects a summary without them.

#### Web env keys read statically (web products)

`import.meta.env[name]` with a computed key made Vite inline every `VITE_*` value from `web/.env` into the production bundle. A computed key is gone from the template.

1. Copy `web/src/vite-env.d.ts`, which declares the product's `VITE_*` keys on `ImportMetaEnv`, so `noPropertyAccessFromIndexSignature` accepts `import.meta.env.VITE_...`.
2. Copy `web/src/api.ts`, and for an authenticated product `web/src/auth.ts`, `web/src/delete-profile.ts`, `web/src/sign-in/local.ts` and `web/src/sign-in/production.ts`, or make the same change by hand: every `import.meta.env['VITE_X']` and `import.meta.env[name]` becomes `import.meta.env.VITE_X`. Add any product key to `vite-env.d.ts`.
3. Clerk and WorkOS products copy `web/scripts/check-production-bundle.mjs` and `check-production-bundle.test.mjs`. `check:bundle` now builds with a random `VITE_BUNDLE_CHECK_CANARY` value and fails with `inlined Vite env object` when the bundle carries it.

#### Organization header and plan retry

`baukit-http` exports `X_ORGANIZATION_ID` and allows it in the default CORS request headers. A product that added `x-organization-id` through `with_additional_allowed_headers` (AHP) may drop it. `useOrganizationPlan` returns `OrganizationPlan`, the old state plus `retry()`, which reads the plan again. It still passes to `useFeature` and `LockedFeature` as `organization`. A product that rebuilt its backend object to retry calls `retry()` from its `unavailable` view instead.

#### Mobile scripts, app-config tests and Maestro paths (mobile products)

1. Copy `mobile/eslint.config.js`, or add the `preserve-caught-error` and `@typescript-eslint/no-dynamic-delete` rules and the `scripts/**/*.{js,mjs,cjs}` block with `tseslint.configs.disableTypeChecked`, and add `scripts` to the `lint` script in `mobile/package.json`.
2. Clerk and WorkOS products copy `mobile/scripts/check-production-bundle.mjs`, which passes the caught error as `cause`, `mobile/scripts/sign-in-config.test.mjs`, which keeps only the Metro cases, and the new `mobile/src/app-config.test.ts`, which runs the app-config cases under Jest. A product that already moved those cases to its unit runner (Schlauzug, Hebkit, Solo Leveling, Redemut, Tiefgang) keeps its own test and drops the template's app-config file.
3. Strict products add `"mobile/.maestro/**" = ["mobile", "android-build", "android-e2e"]` to `[quality.affected]` in `baukit.toml`, with their own lane names when the Android steps run in another lane (KS.18), and copy `scripts/gate.test.py`.

## v0.18.2

Last verified release train: `v0.18.2` (patch train on top of `v0.18.1`). `baukit-suite` and `@baukit/suite-client` read the hub's organization SSO and directory-sync state. The templates keep local sign-in and loopback URLs out of production bundles, bound every adb call in the Android QA setup, and warn about stale `.env` keys. Doctor accepts nested QA Keycloak port variables and the Expo web shape. The templates drop all 24 Expo SDK 57 `minimumReleaseAgeExclude` entries. At 09:41 UTC on 2026-11-06, a fresh render installed without them and pnpm listed none as inside its one-day window.

### Upgrade steps

Follow the sections in this order, starting from a product on 0.18.1. Render a fresh product with the same capabilities (`baukit new`) at v0.18.2 to copy files from.

#### Bump the pins

1. Set every `baukit-*` crate to `=0.18.2` and every `@baukit/*` package to `0.18.2`, and set `baukit.toml`'s template version to `0.18.2`.
2. Strict products move the Baukit clone in `scripts/quality-gate.sh` to `git clone --branch v0.18.2`. A product whose gate also pins `baukit_commit` sets it to the `v0.18.2` tag's commit in the same change. Two products missed these on 0.18.1, and their gates kept testing the old templates.
3. Mobile products delete the `minimumReleaseAgeExclude` list from `mobile/pnpm-workspace.yaml`. The Expo SDK 57 patch releases are past pnpm's one-day window.
4. Refresh `backend/Cargo.lock` with `cargo update -p baukit-core` (or any pinned crate) and the pnpm lockfiles with `corepack pnpm@12.9.1 install`.

#### Organization SSO and directory state

`OrganizationSummary` in `baukit-suite` gains the required `sso_connection_active` and `directory_sync_active`, and `@baukit/suite-client`'s `OrganizationSummary` gains `ssoConnectionActive` and `directorySyncActive`. Both come from the hub's `GET /platform/v1/organizations/{id}` since Schlüsselbund C34.F, so a product reads organization summaries only from a hub with C34.F. A product that builds `OrganizationSummary` literals in tests sets both fields.

#### Local sign-in out of production bundles

The `.18` product tasks found four ways local sign-in or local URLs reached a production bundle, and the templates close each one. A Clerk or WorkOS product takes these steps (an OIDC product takes steps 1, 2 and 5):

1. Copy `mobile/scripts/qa-build-flag.mjs` and `mobile/scripts/qa-build-flag.test.mjs` from a fresh render, and the new `metro.config.js` head: it imports `expo/metro-config.js`, calls `assertQaBuildFlagFromShell(import.meta.dirname)` and uses `import.meta.dirname` instead of `__dirname`. Metro now refuses to bundle when `mobile/.env`, `.env.local` or a `.env.<mode>[.local]` file sets `BAUKIT_QA_BUILD`, because Expo loads those files into every export (LEI.18, HEB.18). A product with its own flag name, such as Hebkit's `HEBKIT_QA_BUILD`, changes `QA_BUILD_FLAG`. The QA scripts keep exporting the flag in their shell.
2. Remove `BAUKIT_QA_BUILD` from `mobile/.env.example` and `mobile/.env` if it is there. `baukit doctor` now rejects it in both.
3. In `mobile/app.config.ts`, copy `localSignInBuild` and pass `localOidcIssuer` only when it is true (`NODE_ENV` is not `production`, or `BAUKIT_QA_BUILD=1`), and set `apiBaseUrl` only when `EXPO_PUBLIC_API_URL` is set (SLS.18, KS.18). The loopback fallback stays in `mobile/src/api.ts` and `mobile/src/delete-profile.ts` under `__DEV__`. A product without sign-in makes the same `apiBaseUrl` change, and its `mobile/src/api.ts` now falls back to the loopback API only under `__DEV__`. Doctor no longer looks for the API port in `mobile/app.config.ts`.
4. Copy `mobile/scripts/check-production-bundle.mjs`, `check-production-bundle.test.mjs` and the new `mobile/scripts/sign-in-config.test.mjs`, and add both new tests to the `test` script in `mobile/package.json`. The bundle check now exports with `--clear` in a private Metro cache (a cached transform hid a leak in KS.18), keeps `EXPO_PUBLIC_LOCAL_OIDC_ISSUER` and the rest of `mobile/.env`, falls back to the `.env.example` values when no `mobile/.env` exists, scans the Expo web export when the Expo app hosts the PWA, and scans files copied from `mobile/public/` as unmapped code instead of failing on their missing source map.
5. Standard-profile products copy the `Check that production bundles carry no local sign-in` step into the `mobile` job of `.github/workflows/ci.yml`. Strict products already run it as `mobile-bundle`.

The web template needs no change: `web/vite.config.ts` doesn't call `loadEnv`, and `web/src/auth.ts` keys on Vite's `--mode`. A product whose `vite.config.ts` reads its QA flag through `loadEnv` (HEB.18) reads it from `process.env` instead.

#### Doctor, bounded adb and setup warnings

1. Copy `scripts/reconcile-env.py` and `scripts/reconcile-env.test.py` from a fresh render. `sh scripts/setup.sh` now prints a warning for each local `.env` key its `.env.example` no longer lists. Delete those keys by hand.
2. Mobile products copy `mobile/scripts/qa/android-adb.py`, `mobile/scripts/qa/android-env.sh` and `mobile/scripts/qa/test_android.py`. Every adb call in the setup loop now times out after `BAUKIT_QA_ADB_TIMEOUT_SECONDS` (default 60). Device probes keep 10 seconds.
3. Doctor now accepts `${BAUKIT_QA_KEYCLOAK_PORT:-${PRODUCT_KEYCLOAK_PORT:-18081}}` when the inner default is the declared port. Products that copied the plain form need no change.
4. An Expo web build (`expo-router` in `web/package.json`) no longer needs `web/src/api.ts`, the OIDC Vite auth files (`web/src/auth.ts`, `web/src/auth.test.ts`, `web/src/local-data.ts`, `web/src/persistence-lifecycle.ts`) or the web auth SDK. Doctor expects the mobile auth SDK (`@clerk/expo` or `@baukit/auth-native`) and `EXPECTED_MOBILE_AUTH_DEPENDENCIES` in `web/package.json` instead. A product that kept those files only for tests may delete them.
5. Doctor runs in the gate now. Strict products copy the `baukit-doctor fast - always -` line into the step list of `scripts/quality-gate.sh` and the `step_baukit_doctor` function, which builds the CLI from the pinned Baukit checkout with `cargo run --locked`. Doctor fails a strict gate that lacks the step. Standard products copy the `baukit-doctor` job into `.github/workflows/ci.yml`. A product that already runs doctor as its own step drops that step to avoid running it twice.

## v0.18.1

Last verified release train: `v0.18.1` (patch train on top of `v0.18.0`). It takes the Expo SDK 57 patch releases, maps a custom Clerk session-token claim with the sign-in time to `Principal::authenticated_at` so the erasure fence lets a Clerk user who erased a profile sign in again, and fixes three doctor gaps: declared QA Keycloak ports, a declared local PKCE callback port, and template gate steps the product's gate lacks. `baukit-auth` adds `CLERK_SIGN_IN_TIME_CLAIM`; no other crate or package API changed. Baukit's `rust/deny.toml` ignores RUSTSEC-2026-0440 (astral-tokio-tar, unmaintained, no patched release), which only testcontainers 0.28.0 in `baukit-test` pulls in.

### Upgrade steps

Follow the sections in this order.

#### Bump the pins

Set every `baukit-*` crate to `=0.18.1` and every `@baukit/*` package to `0.18.1`, set `baukit.toml`'s template version to `0.18.1`, then refresh `backend/Cargo.lock` and the pnpm lockfiles.

#### Expo SDK 57 patch releases

Expo published SDK 57 patch releases on 2026-11-05, and `expo-doctor` fails a 0.18.0 render since then. A mobile product bumps `expo` to 57.0.32 and `expo-router` to 57.0.30 in `mobile/package.json`, `expo-constants` to 57.0.23 in the `catalog` of `mobile/pnpm-workspace.yaml`, and `@expo/metro-runtime` to 57.0.18 when the Expo app hosts the PWA. pnpm refuses releases younger than a day, so the template lists the 24 exact `name@x.y.z` releases from the SDK 57 patch set in `minimumReleaseAgeExclude`. Copy that list from a fresh render, run `corepack pnpm@12.9.1 --dir mobile install` and check that `pnpm dlx expo-doctor` passes 21/21.

#### Clerk sign-in time for the erasure fence

Clerk session tokens carry no `auth_time`, so baukit-erasure fenced every Clerk session of a user who erased a product profile, for good. `baukit-auth` adds `CLERK_SIGN_IN_TIME_CLAIM` (`session_created_at`), and `ClerkVerifier` maps that claim, Unix seconds, to `Principal::authenticated_at`. The fence itself is unchanged. A token without the claim stays fenced, and a malformed one fails with `MalformedToken`. The OAuth and MCP verifiers are unchanged. A Clerk product takes these steps:

1. In the Clerk dashboard, open **Sessions**, **Customize session token** and merge `{"session_created_at": "{{session.created_at}}"}` into the claims, in every Clerk instance (development and production).
2. Copy the `clerk sign-in time` mapper from a fresh render's `keycloak/realm.json` into each Clerk-shaped client, and the new assertions in `backend/tests/local_realm.rs`. The mapper is Keycloak's built-in `oidc-usersessionmodel-note-mapper` on the `AUTH_TIME` session note, so local tokens carry the same claim.
3. Re-render `docs/auth-providers.md`, then recreate the local Keycloak so it imports the realm again.

#### Unmaintained astral-tokio-tar advisory

A product that runs `cargo deny` adds the same single-ID entry to its `deny.toml` `[advisories] ignore`, `{ id = "RUSTSEC-2026-0440", reason = "astral-tokio-tar is unmaintained with no patched release; reached only through testcontainers 0.28.0, used by baukit-test as test support; remove when testcontainers ships testcontainers-rs#1006" }`, and removes it once a testcontainers release drops astral-tokio-tar.

#### Doctor: declared QA Keycloak and callback ports, template gate steps

For Clerk and WorkOS mobile products, doctor's QA Keycloak check accepts `${BAUKIT_QA_KEYCLOAK_PORT:-<port>}` on the bundled 18081, on `ports.keycloak` and on each `ports.keycloak_extra` port, and still rejects any other port. `[ports] local_callback = { host = <port> }`, default 18890, sets the realm's `<app>-local` PKCE callback, and doctor checks that client's redirect URIs against it instead of the `ports.web` browser ports. Strict products: doctor renders the template gate's step table for the declared capabilities and fails when `sh scripts/quality-gate.sh --list-steps` lacks one of its steps. A product on 0.18.0 takes these steps:

1. Run `baukit doctor`. A missing template step, such as `expo-doctor` in Eigenruhe, gets copied from a fresh render's `scripts/quality-gate.sh` and `docs/quality-gate.md`.
2. If the product moved the local PKCE callback off 18890, declare it as `ports.local_callback`.
3. A product whose QA reuses its dev Keycloak (Tiefgang on 8281) or runs its own (Eigenruhe on 24003) declares that port as `ports.keycloak` or in `ports.keycloak_extra`, then sets `auth = "clerk"` (or `"workos"`) in `baukit.toml` and runs `baukit doctor` again. The provider checks and the `web-bundle` and `mobile-bundle` steps now apply.

## v0.18.0

Last verified release train: `v0.18.0` (breaking minor train on top of `v0.17.1`). Production has no Keycloak any more: BK21 removes the operator base and the local cluster's Keycloak, and Keycloak stays for local development, E2E and QA only. `HubClient` can send a hub `sbsvc_` service token. Clerk and WorkOS web and Expo apps sign in through one port, with the BK19 Keycloak realm locally and the provider SDK in production (BK24, BK22). Organizations are plan kit subjects (BK25). The lane gate shares one core budget, and the native QA scripts gain stall, Wi-Fi and snapshot fixes (BK27).

### Breaking changes

A product moving from 0.17.1 hits these first:

- `deploy/platform/keycloak/` and `deploy/smoke/headless-pkce-login.py` are gone, and `deploy/smoke/k3d-smoke.sh` takes `BAUKIT_SMOKE_TOKEN_COMMAND` instead of the OIDC login settings (BK21).
- `scripts/keycloak_policy.py` accepts only `--environment-class development` (BK21).
- `baukit-suite` `platform`: `HubClientConfig.token_url` becomes `Option<String>` and the struct gains `token` (BK21). `require_feature` takes `impl Into<FeatureSubject>` and returns `FeatureGrant`, `PlanError` gains two variants, and `AccountEntitlement` gains `organization` (BK25).
- `@baukit/purchases-core`: `Paywall['kind']` gains `organization_managed` (BK25).
- WorkOS Expo apps rename `EXPO_PUBLIC_OIDC_ISSUER` and `EXPO_PUBLIC_OIDC_CLIENT_ID` to the three `EXPO_PUBLIC_WORKOS_*` settings (BK22).
- `@baukit/sync-client` always passes a `signal` to `fetch`, so tests that compare the whole request init must allow it (BK27).

### Upgrade steps

Follow the sections in this order.

#### No production Keycloak (BK21)

BK21 drops Baukit's production Keycloak. Breaking for platform overlays and the k3d smoke: `deploy/platform/keycloak/` (the Keycloak operator 26.8.0 base with its vendored CRDs, the realm import example and the realm migration Job example) is gone, as are the local cluster's `local-keycloak` and `local-realm` Flux Kustomizations and their components. Production signs in with Clerk or WorkOS; Keycloak runs only in each product's compose stack, its E2E and the QA Keycloak on 18081.

- A private overlay that lists `deploy/platform/keycloak` removes that entry, its `keycloak` namespace, the `keycloak-db-credentials` and `keycloak-bootstrap-admin` Secrets and the `keycloak` database and role before it pins `v0.18.0`.
- `deploy/smoke/headless-pkce-login.py` is gone. `deploy/smoke/k3d-smoke.sh` drops `BAUKIT_SMOKE_ISSUER`, `BAUKIT_SMOKE_OIDC_CLIENT_ID`, `BAUKIT_SMOKE_OIDC_USERNAME`, `BAUKIT_SMOKE_OIDC_PASSWORD` and `BAUKIT_SMOKE_RESOLVE_HOST` and requires `BAUKIT_SMOKE_TOKEN_COMMAND`, an executable that writes one access token to `$BAUKIT_SMOKE_ACCESS_TOKEN_FILE`. The harness sends it to `BAUKIT_SMOKE_API_CHECK_PATH` and adds it to the log leak scan. Architecture Health Platform's `scripts/k3d-smoke.sh` is the one known caller.
- `scripts/keycloak_policy.py` accepts only `--environment-class development`. Re-render it with `scripts/tests/test_keycloak_policy.py`, delete `scripts/tests/fixtures/production-realm.json` and `production-policy.json`, and drop any `--environment-class production` call.
- The `oidc` flavor stays. It verifies any OIDC issuer; its bundled realm is for development, and a deployed `oidc` product supplies the `IdentityAccountDeleter` for its own issuer. Re-render `docs/auth-providers.md`.
- The local platform fixture verifies tokens from its compose Keycloak on the host (`AUTH__JWKS_URI` through `host.k3d.internal:8091`). Recreate a running `baukit-local` cluster; `platform-up.sh` no longer writes Keycloak credentials.

#### Static hub service token in HubClient (BK21)

`HubClient` sends a hub-issued `sbsvc_` platform service token (from the hub's `service-token create`) as the bearer when `HubClientConfig.token` is set. It never calls a token endpoint, never refreshes, and maps a 401 to `HubError::Unauthorized` without a retry. Breaking for `baukit-suite` `platform`: `HubClientConfig` gains `token: Option<Secret<String>>`, and `token_url` becomes `Option<String>`, so struct literals wrap it in `Some(...)` and add `token: None` or `..Default::default()`. Validation rejects a config with neither a token nor client credentials, and one with both. Client-credentials mode behaves as before.

To move a product to the service token, set `<PREFIX>__PLATFORM_CLIENT__TOKEN` from a Secret reference in the deploy values (never a value) and remove `PLATFORM_CLIENT__TOKEN_URL`, `PLATFORM_CLIENT__CLIENT_ID` and `PLATFORM_CLIENT__CLIENT_SECRET`. Re-render `.env.example`, `deploy/values.yaml` and `docs/platform.md` for the commented examples.

#### One sign-in port for web and Expo (BK24)

BK24 moves Clerk and WorkOS web and Expo apps onto one sign-in port. Local and QA builds sign in against the product's BK19 Keycloak realm with PKCE, and production builds use the provider SDK. `@baukit/api-runtime` adds `unverifiedSignInSessionFromJwt`, which maps Clerk `sub`, `sid` and `o` or WorkOS `sub`, `sid`, `org_id`, `role` and `permissions` to one `SignInSession`. BK24 changes no crate API.

To upgrade a Clerk or WorkOS product, delete its own sign-in wiring in `web/src/auth.ts` and `mobile/src/auth.ts` and copy from a fresh render:

- `src/sign-in/` (`port.ts`, `local.ts`, `production.ts`, their tests and, on web, `provider-client.ts`) in `web` and `mobile`, plus the new `src/auth.ts`.
- On web, the `import.meta.env.MODE` switch in `src/auth.ts` and `server.deps.inline` in `vitest.config.ts`. On mobile, the `metro.config.js` resolver that swaps `local.ts` for `production.ts` when `NODE_ENV=production` and `BAUKIT_QA_BUILD` is not `1`, and the `__DEV__` guard on the loopback API fallback in `src/api.ts` and `src/delete-profile.ts`.
- `scripts/check-production-bundle.mjs` and its test in both apps, the `check:bundle` and `test` package scripts, and the `web-bundle` and `mobile-bundle` steps in `scripts/quality-gate.sh` and `docs/quality-gate.md`.
- `VITE_LOCAL_OIDC_ISSUER` and `EXPO_PUBLIC_LOCAL_OIDC_ISSUER` in the `.env.example` files.
- The mobile QA scripts (`services.sh`, `build-android.sh`, `build-ios.sh`, `android-env.sh`, `docker-compose.qa.yml`, `test_android.py`), which now start Keycloak and point the API at the local realm, and `.maestro/smoke.yaml`, which signs in with the local test account.
- `web/e2e/stack/sign-in.spec.ts`, which now also runs for provider products with a backend.

Components that used the provider SDK directly read the session from the port instead.

#### WorkOS sign-in for Expo apps (BK22)

BK22 finishes WorkOS sign-in for Expo apps. `@baukit/auth-native` adds `NativeOidcConfig.refreshOnRestore`, false by default and on in `createWorkOsNativeClient`: `initialize()` refreshes an expired stored session before it resolves, ends it on a rejected refresh and keeps it on a network error. The WorkOS mobile config moves from `EXPO_PUBLIC_OIDC_ISSUER` and `EXPO_PUBLIC_OIDC_CLIENT_ID` to `EXPO_PUBLIC_WORKOS_CLIENT_ID`, `EXPO_PUBLIC_WORKOS_API_HOSTNAME` (default `api.workos.com`) and `EXPO_PUBLIC_WORKOS_REDIRECT_URI` (default `<app>://oauth`), and the `extra` keys become `workosClientId`, `workosApiHostname` and `workosRedirectUri`. Clerk mobile apps lose the unused `oidcIssuer` and `oidcClientId` extras. The mobile `check:bundle` fails on an `sk_test_` or `sk_live_` key, and `baukit doctor` checks that a WorkOS mobile app declares and reads the three settings and carries no `sk_` key in `mobile/.env` or `mobile/.env.example`. BK22 changes no crate API.

To upgrade a WorkOS Expo product, copy `mobile/app.config.ts`, `mobile/src/sign-in/production.ts`, `mobile/src/sign-in/production.test.tsx` and `mobile/scripts/check-production-bundle.mjs` with its test from a fresh render, rename the two env values in `.env.example`, `.env` and the release build environment, and add `<app>://oauth` as a sign-out redirect in the WorkOS dashboard. A Clerk product copies only the bundle check and `app.config.ts`.

#### Organizations as plan kit subjects (BK25)

Organizations are plan kit subjects (BK25). Breaking for `baukit-suite` `platform`: `require_feature(plan, subject, key)` takes `impl Into<FeatureSubject>` and returns `FeatureGrant { tier, features, revision, organization_id }` instead of `AccountEntitlement`. `Some(subject)` and `None` still compile. `PlanError` gains `OrganizationNotAccessible` and `OrganizationIdInvalid`, so exhaustive matches need two arms. `AccountEntitlement` gains `organization`, and struct literals need `organization: None`. The account body on the wire is unchanged. A product enables organizations with `HubAccountPlan::with_organizations(Arc::new(HubOrganizations::new(hub)))`, has its authentication layer insert `PlanOrganization(id)` for organization-owned API tokens, sends `X-Organization-Id` from clients, and gates with the `FeatureSubject` extractor in handlers or `FeatureSubject::Organization(id)` in workers. Errors: 403 `organization_not_accessible` for an unknown organization, a non-member or a token naming another organization, all with the same body; 400 `organization_id_invalid`; `feature_not_entitled` and 503 `entitlements_unavailable` as for accounts. Grants and memberships are cached for 60 seconds. `@baukit/purchases-core` adds `organizationEntitlement`, `useOrganizationPlan`, `organizationFeatureAccess`, the `organization` option of `useFeature` and `LockedFeature`, and the `organization_managed` paywall, so exhaustive switches over `Paywall['kind']` need a new arm. [platform-receiver.md](platform-receiver.md#organizations-as-subjects) describes the subject.

What a product deletes when it moves to the kit's subject:

- Architecture Health Platform (AHP.ORG): `PlanEntitlements` and its `EntitlementPort` (`privacy_entitled`, `automation_entitled`), `PlanFeature` and the `OrganizationPlans` port with its `PostgresHubOrganizations::effective_plan` adapter and `organization_plans` in `suite_platform`, the organization entitlement that `suite_account.rs` builds from `effective_plan` (the kit's `GET /me/entitlement` with `X-Organization-Id` serves it), and the `billing: null` the web passes in `web/src/suite-account.ts`. Its gate tests move to `require_feature` with `FeatureSubject::Member` and `FeatureSubject::Organization`. Organization sync, seats and memberships stay, because AHP uses them for tenancy, not for gating.
- Runtime Analyzer (RA.ORG): `OrganizationPlan` and `tenant_plan` as a plan source (`finops-ports` `organizations.rs`, `finops-postgres` `organizations.rs`, `HubPlatform::tenant_plan`), the plan field on `FinopsPrincipal`, and the product's `feature_not_entitled` helper and `AccessDenial::FeatureNotEntitled` in `routes/plan.rs`, whose callers move to `require_feature`. The `tenant_entitlements` capability checks in `finops-postgres` are product data, not plan gates, and stay. Its `X-Organization-Id` header stays and becomes the kit's `ORGANIZATION_HEADER`; tenant selection, seat and suspension checks stay.

A BK24 `SignInSession.organization.id` is the sign-in provider's organization ID, not the hub organization UUID that `X-Organization-Id` takes, so a client passes the hub ID it got from the product backend to `useOrganizationPlan`.

#### Lane gate and native QA fixes (BK27)

BK27 fixes gaps in the lane gate and the native QA scripts. Re-render `scripts/gate.py`, `scripts/quality-gate.sh`, `docs/quality-gate.md` and `mobile/scripts/qa/` (`android-env.sh`, `android-adb.py`, `run-maestro.sh`, `build-android.sh`, the new `native-jobs.init.gradle` and `test_android.py`), then run `baukit doctor`. The gate now holds one core budget for all lanes. A step takes its share from `BAUKIT_GATE_CORES` and gives it back when it ends, and the share also caps Gradle (`BAUKIT_QA_GRADLE_WORKERS`) and CMake job pools (`BAUKIT_QA_NATIVE_JOBS`). `android-e2e` waits for at least `BAUKIT_GATE_EMULATOR_CORES` cores, 2 per emulator by default. `quality.light`, default `[]`, lists lanes or steps that take no share. Products that capped Gradle or CMake in their own gate, like Hebkit, can drop that. Doctor now checks `quality.affected` and `quality.light` against the steps that `sh scripts/quality-gate.sh --list-steps` prints, so product-only step names work. Emulator ports form blocks of 8 from 5556, and doctor prints each product's range and rejects one that runs past its block. Kassensturz (5594) and Lesezeichen (5674) fail that check, and Hebkit and Kassensturz, Schlüsselbund and Tiefgang overlap. `docs/quality-gate.md` has the table and the free blocks, so move to a free block on the next task. Products without `quality.android_emulator_port` get a new default from their name. `[ports] keycloak_extra` lists further loopback realm ports doctor accepts in issuers. `quality.android_wifi`, default false, starts every emulator with `-feature -WiFiPacketStream` and waits for a Wi-Fi route within `BAUKIT_QA_WIFI_TIMEOUT_SECONDS` (90). Hebkit, whose offline flows toggle Wi-Fi, should set it to true. Tiefgang should leave it off, because its second emulator lost the adb connection under the flag in three of three runs. The snapshot name now includes the emulator features, so the first boot after the upgrade is cold and saves a new snapshot. The prune keeps snapshots that running emulators of the AVD loaded and the newest finished older one. `run-maestro.sh` stops a run without output or debug-file changes for `BAUKIT_QA_MAESTRO_STALL_SECONDS` (300) and fails it with exit 124, keeping `maestro-debug/` and its new `maestro-output.log`. `@baukit/sync-client` `SyncTransport` gains `requestTimeoutMs`, 30000 by default, and now always passes a signal to `fetch` or the prebuilt request function. Tests that compare the whole request init must allow the `signal` field. Product tests that assert the exact Gradle command line, like Tiefgang's `scripts/tests/test_android_templates.py`, must clear `BAUKIT_QA_GRADLE_WORKERS` and `BAUKIT_QA_NATIVE_JOBS` in their setup, because the gate now sets both and the Gradle call gains `--max-workers`.

### Earlier trains

Before that, `v0.17.1` (patch train on top of `v0.17.0`). The Android QA emulator now runs `cmd package bg-dexopt-job` after its first cold boot and waits until the guest uses less than one core, up to `BAUKIT_QA_SETTLE_TIMEOUT_SECONDS` (600 by default), before it saves the quick-boot snapshot. The snapshot is named `baukit-clean-<hash>` after the system image, the AVD's `hw.*` settings, the GPU mode and the template version, and a new save deletes the AVD's older `baukit-clean*` snapshots. The emulators render with `-gpu swiftshader_indirect`. No crate or package API changed. A mobile product copies `android-adb.py` from a v0.17.1 render and ports the snapshot block, the `gpu_mode` and the `.ready` and lock handling into its `android-env.sh`; its first gate after the upgrade cold-boots once. [strict-quality-profile.md](strict-quality-profile.md#v0171) lists the steps.

Before that, `v0.17.0` (minor train on top of `v0.16.0`). The strict gate now runs in parallel lanes through `scripts/gate.py`, runs each suite once, keeps lane logs and `timings.txt` in `.quality-gate/`, and takes `--affected BASE` to run only the lanes `[quality.affected]` maps to changed paths. Shared paths and unmatched files fall back to the full gate. Non-desktop-Chromium Playwright projects skip `@logic` specs, and the protected `qa-*` specs still run on every project. The Android QA emulator boots from a `baukit-clean` snapshot after its first clean boot, checks health between Maestro flows and listens on `quality.android_emulator_port`. `quality.android_emulators`, 2 by default, boots that many emulators on consecutive even ports and shards Maestro flows across them with `--shard-split`. `quality.gate_budget_minutes` defaults to 10 and 5 minutes for new products; mid-size repositories set 20 and 10, large ones 30 and 15. No crate or package API changed. A product's `.T` copies `scripts/quality-gate.sh`, `scripts/gate.py`, `scripts/gate.test.py` and `docs/quality-gate.md` from a fresh render, adds `/.quality-gate/` to `.gitignore`, adds `shared`, `[quality.gate_budget_minutes]` and `[quality.affected]` to `baukit.toml`, sets `quality.android_emulator_port` and `quality.android_emulators` when it has mobile, picks its budget tier, renames `BAUKIT_QA_ANDROID_EMULATOR_PORT` to `BAUKIT_QA_ANDROID_PORT`, makes `make ci` run only the gate, drops thread clamps and `--runInBand`, tags logic-only Playwright tests `@logic`, and copies the Android QA scripts and `preflight.sh`. [strict-quality-profile.md](strict-quality-profile.md#v0170) lists the steps.

Before that, `v0.16.0` (breaking minor train on top of `v0.15.0`). Clerk and WorkOS products now develop against a provider-shaped Keycloak realm and run their production verifier locally. To adopt it, copy `keycloak/`, the compose `keycloak` service and Makefile Keycloak targets, `config/local.toml` and `backend/tests/local_realm.rs` from a fresh generation. Local config sets `auth.issuer` and `auth.jwks_uri` to `http://localhost:<keycloak port>/realms/<app>`, Clerk products list `<app>-local` and the web and mobile client IDs in `auth.authorized_parties`, and WorkOS products set `auth.client_id = "client_<app_crate>_local"`. Production keeps its issuer, JWKS and client settings through environment variables. The realm's public clients use only the `basic` scope with `fullScopeAllowed` off and hardcoded-claim mappers: Clerk adds `v` and `o`, WorkOS adds `client_id`, `org_id`, `role` and `permissions`; no client sends `aud`. No verifier constructor changed. `baukit-auth` adds `ClerkMcpVerifier` for Clerk OAuth tokens from dynamically registered MCP clients and `VerificationError::MissingScope`. `baukit-mcp` `McpConfig` adds `required_scopes`; it is not `#[non_exhaustive]`, so struct literals need `..Default::default()` or `required_scopes: Vec::new()`. Clerk products may now leave `mcp.oauth_client_id` unset when they set `mcp.required_scopes`, and the generated `mcp_verifier` picks `ClerkMcpVerifier` then. `baukit-test` adds `KeycloakTestContainer` and `KeycloakLogin`.

Before that, `v0.15.0` (breaking minor train on top of `v0.14.1`). The plan catalog helpers in `baukit-suite` `platform` (`catalog_feature_keys`, `feature_key_drift`, `assert_feature_keys_match`) and `@baukit/suite-client` (`catalogFeatureKeys`, `featureKeyDrift`, `assertFeatureKeysMatch`) now count keys a plan grants through `organizationProducts` as well as `includedProducts`; a missing or `null` list grants nothing. The recorded `fixtures/plans/v1/catalog.json` is the hub's at Schlüsselbund C23 (`fd2a2a3`), whose business plans grant `hebkit.trainer` and `eigenruhe.studio` to organizations. A product whose contract test copies the new fixture must declare every organization-only key it receives, for example `hebkit.trainer` in Hebkit. Breaking for `baukit-test`: `check_identity_erasure_conformance(adapter, routes, subject, foreign_subject)` takes `&IdentityErasureRoutes`, and `IdentityErasureAdapter` replaces `delete`, `status` and `resolve` with one `request(subject, IdentityErasureRequest { method, path, idempotency_key })`. Products pass `&IdentityErasureRoutes::default()` for the template's `DELETE /me`, `GET /me/erasures/{operation}` and `GET /me`, or their own routes, and implement `request` by sending the given method, path and optional `Idempotency-Key` through their router. Generated backends get the new adapter shape in `backend/tests/identity_erasure_conformance.rs`. `@baukit/api-runtime` `createProfileErasureClient` accepts optional `routes: { eraseMethod, erasePath, statusPath }` with `templateProfileErasureRoutes` as the default, so existing callers need no change. Baukit's own `make ci` runs the browser suites through `scripts/with-loopback-network.sh`; nothing changes for products.)

Before that, `v0.14.1` (patch train on top of `v0.14.0`). `v0.14.0` was tagged but never published, because the release check failed on the `minimal-api` example, which still expected `validation_failed` for malformed JSON. Consumers move from 0.13.0 straight to 0.14.1. The 0.14.1 train fixes the example, runs the release verification in `make ci`, and carries every change the 0.14.0 entry below lists.

`v0.14.0` (tagged, unpublished; breaking minor train, needs the Schlüsselbund hub with C21 (`f8bc162`) for the bucketed credit balance and C27 (`aa3204d`) for the `billing` ticket claim. `baukit-suite` `platform` `CreditBalance` and `@baukit/suite-client` `CreditBalance` read the bucketed balance `{suiteSubject, wallet, revision, total, grant: {available, expiresAt|null}, packs: [{available, expiresAt}]}`. `available`, `pools`, `CreditPool`, `grant.granted`, `grant.used` and `packs[].remaining` are gone, and `HubCredits::balance` rejects the old shape as `HubError::InvalidResponse`, so products move to this train and the C21 hub together. Products read `total` instead of `available`, `packs[].available` instead of `remaining`, and `grant.expires_at` for the grant expiry. The recorded `creditBalance` fixture comes from the hub at `f8bc162`. `TicketClaims` and the `@baukit/suite-client` claims gain the optional `billing` claim, which the verifiers rejected as unknown before; struct literals need `billing: None`. The claim only arrives from a hub with C27, which adds `includeBilling` to `POST /platform/v1/entitlements/tickets` on top of the C18 claim from `c48f8af`. `baukit-suite` `platform` adds the plan kit: the `AccountPlan` port with `StandaloneAccountPlan` and `HubAccountPlan`, `require_feature`, and `account_plan_router` for `GET /me/entitlement` and `GET /me/credits` with the error codes `suite_identity_missing`, `entitlements_unavailable`, `credits_unavailable` and `feature_not_entitled`. `PlatformTicketSource` sends `includeBilling: true`. Both `baukit-suite` and `@baukit/suite-client` add a feature key contract helper over `fixtures/plans/v1/catalog.json` from hub `c08ed7c`. `@baukit/purchases-core` changes `VerifiedEntitlement` to `{mode, suiteSubject, tier, features, revision, plansUrl, billing}`, `CreditBalanceSnapshot` to the bucketed balance, `useFeature` to return a `FeatureAccess` status instead of a boolean and `PurchasesProvider` to require `productId`; it adds `LockedFeature`, `paywallFor`, `plansLink`, `featureAccess` and `createAccountPlanBackend`. Generated mobile apps pass `productId` to `PurchasesProvider`. The train also removes retired compatibility code. `baukit-http` drops `JsonRejectionMode`, `HttpOptions::with_json_rejection_code` and the single-code `json_rejection_code()`; call `with_json_rejection_codes` or keep the defaults, and note that `json_rejection_codes()` returns `&JsonRejectionCodes` and default bad-JSON rejections answer `400 invalid_json` (with `payload_too_large`, `unsupported_media_type` and `validation_failed` for the other rejections). `baukit-test` drops `trimmed_text_length` and `compact_document_bytes`; use `trimmed_unicode_scalar_count` and `compact_json_utf8_bytes` from `baukit_core::limits`. `baukit-runtime` drops the `Shutdown` alias; use `ShutdownToken`. `baukit doctor` no longer explains retired MCP manifest shapes (`[mcp]` tables, TypeScript MCP selections), which now fail as unknown keys; follow `docs/platform/remote-mcp.md`. `@baukit/sync-client` drops `lastSyncAt` from the snapshot, `setError` and the positional `hydrate` overload; conformance adapters provide `pendingId` and `applySubmittedBatchOutcome` and delete `markAcknowledged` and `recordRejected`. `@baukit/data-contracts` no longer claims unowned legacy stores: `LegacyStoreOwnership`, `LegacyStoreInspection`, `inspectLegacy` and `legacyStoreName` are gone and `ResolvedScopedStore` is `{ storeName }`. Generated web QA submit targets in `e2e/qa.config.ts` take `fields` and `invalidField` instead of the single-field `QaLegacySubmitField` shape. The Baukit chart adds optional `api.podDisruptionBudget` and `worker.podDisruptionBudget` values, off by default. Before that, `v0.13.0` (breaking minor train, needs the Schlüsselbund hub with C16. `baukit-suite` `platform` adds `OrganizationSummary::name` (`Option<String>`), the organization's display name, and `HubOrganizations::organizations_for_subject`, which pages `GET /platform/v1/accounts/{suiteSubject}/organizations` through `SubjectOrganizationPage` with `SubjectOrganization` items and `next_cursor`. `OrganizationSummary` is not `#[non_exhaustive]`, so struct literals and exhaustive patterns in products need `name`; add `name: None` or the display name. `HubClient` now percent-encodes every path segment, so `|` in a suite subject goes out as `%7C`, `HubCredits::balance` included. Product tests that match a raw `|` in a hub request path must match `%7C` instead; the hub decodes both forms. An older hub omits `name`, which decodes as `None`, and returns 404 for the subject lookup. `@baukit/suite-client` adds the optional `name` and the `OrganizationRole`, `SubjectOrganization` and `SubjectOrganizationPage` types. The recorded fixtures add `organizationSummaryWithoutName`, `subjectOrganizationsFirstPage`, `subjectOrganizationsLastPage`, `subjectOrganizationsEmpty` and `productMismatch`. `@baukit/a11y-core` `useFocusTrap` no longer throws when the document goes away before its deferred focus restore runs, for example after a Jest test resets `globalThis.document`. Before that, `v0.12.2` (`baukit-suite` `platform` adds `HubClient::get_json_with_query` for query parameters and `HubOrganizations::members`, which pages `GET /platform/v1/organizations/{id}/members` through `OrganizationMemberPage` and its `next_cursor`. Both are additive, no trait changes, and `get_json` is unchanged. The recorded fixtures add `organizationMembersFirstPage`, `organizationMembersLastPage` and `invalidPagination`. Baukit CI keeps the Android jobs within runner disk, sets up the NDK by script and fails on Gradle deprecation warnings; no template changes. Before that, `v0.12.1` (adds `@baukit/purchases-core`, a store-neutral purchase flow gated on the backend-verified entitlement, and `@baukit/purchases-revenuecat`, its `react-native-purchases` 10.14.0 adapter. Generated mobile apps wire both and fall back to the no-op port when no RevenueCat key is set, so they behave as before. Doctor does not require the new dependencies in existing products. Before that, `v0.12.0` (breaking minor train. `baukit-suite` `platform` adds `HubClient` with a cached client-credentials token, the `Credits` port with `HubCredits` and `ZeroCostCredits` for standalone mode, `HubOrganizations` for the Schlüsselbund organization summary, and the `credits.changed` and `organization.updated` platform events; `PlatformEventKind` gains two variants, so exhaustive matches need new arms, and the decoder checks the envelope `userId` against each type's natural key. `@baukit/suite-client` mirrors the credit, organization and event types. The `baukit-jobs` runner takes a fresh time for each claim, so jobs enqueued during a poll loop no longer violate `updated_at >= created_at`. Before that, `v0.11.3` (`@baukit/navigation` pads the native profile menu and section picker modal by the safe-area insets, so the sheet clears edge-to-edge system bars on Android, `SectionPicker` accepts optional `insets`, and `NavigationTypography.activeFontFamily` sets the selected entry's font family. Before that, `v0.11.2` (`@baukit/navigation` shows the native navigation focus ring on web only for keyboard focus (`:focus-visible`), not after a pointer click. Before that, `v0.11.1` (`baukit-mcp` uses a verified API token's grants as its effective MCP scopes, so personal access tokens can call the tools their grants match; `with_scopes` still only narrows, OIDC scopes are unchanged and `baukit_auth::Principal::scopes()` stays OIDC-only. Products that already accept API tokens at `/mcp` must review issued grants. `KeycloakIntrospectionPolicy` rejects API-token principals as inactive without sending their secrets to Keycloak. The workspace lockfile takes tokio 1.53.3 and ctutils 0.4.5 for RUSTSEC-2026-0386 and RUSTSEC-2026-0385. Before that, `v0.11.0` (breaking minor train. `baukit-suite` adds the `platform` feature: the `POST /platform/inbound` receiver with HMAC secret rotation, a PostgreSQL inbox with receipt purge, entitlement caches with hub read-through, and entitlement ticket verification; `@baukit/suite-client/entitlements` verifies the same tickets offline. `baukit new --backend --platform` generates the receiver wiring, a daily purge job with `--worker`, erasure wiring and standalone-safe Helm values. `PeerRegistrySettings` takes a per-peer hosted `authorize_url`; `baukit-mcp` adds a `list_suite_links` tool behind the `suite` feature with a required `SuiteOwnerResolver`, and the OIDC template sets `auth.authorized_parties` as the `azp` allowlist. `@baukit/auth-web` adds `SilentTokenBroker` for silent authorization against several realms. `baukit-auth` adds a `VerificationError` variant for an unavailable token store, so MCP identity outages return 503 `identity_unavailable` with `Retry-After`, and `PostgresApiTokenStore` inserts prepared tokens in the caller's transaction with a public name length limit. `baukit-jobs` adds pending-only deduplication with migration 0004. `baukit-egress` adds a streaming response API and `AllowedEndpoint::new` now pins the scheme; `baukit-push` takes an injected guarded client. Generated backend builds key the BuildKit target cache by checkout through `scripts/docker.sh`, and generated mobile apps pin Expo 57.0.31 and expo-router 57.0.29. Before that, `v0.10.32` (the new crate `baukit-data-subject` implements data subject contract v1: discovery at `/.well-known/baukit/data-subject`, machine-client erasure and export operations with `Idempotency-Key` replay, Ed25519 DSSE receipts whose `keyid` is the SHA-256 of the SPKI DER, and cross-language vectors in `fixtures/data-subject/v1/`; clients depend on it with `default-features = false`, and the default `server` feature adds the PostgreSQL store, routes, jobs and purge. `baukit-erasure` adds `ErasureService::fence_as_operator_in_transaction`. `baukit-test` adds `DataSubjectTargetServer` and `check_data_subject_conformance` behind its `data-subject` feature. The workspace lockfile takes webpki-roots and webpki-root-certs 1.0.10 and rustls-webpki 0.103.16 for RUSTSEC-2026-0381 to 0383. Before that, `v0.10.31` (`@baukit/a11y-core` preserves the original overlay opener across StrictMode effect replays and focus ref changes, so closing an overlay restores focus to its trigger and each separate opening captures a new opener. Before that, `v0.10.30` (`@baukit/a11y-core` keeps focus moved outside a closing overlay and avoids disconnected openers, with `restoreFocus` and `returnFocus` callbacks on `useFocusTrap` and `useOverlayA11y` to opt out or choose a fallback. Before that, `v0.10.29` (generated mobile apps pin Expo 57.0.30, expo-auth-session 57.0.16, expo-constants 57.0.22, expo-linking 57.0.13, expo-router 57.0.28 and @expo/metro-runtime 57.0.17; `@baukit/navigation` accepts six main items plus the profile; seven 44-pixel targets fit the 320-pixel compact bar. The workspace lockfile takes rustls 0.23.46 for RUSTSEC-2026-0346 to 0349; products run `cargo update -p rustls` to clear the same cargo deny failure. Before that, `v0.10.28` (`baukit-test` labels every container it starts with its process, binary, worktree, PID namespace and host, enables the testcontainers watchdog so SIGINT, SIGTERM and SIGQUIT remove containers, and `scripts/reap-test-containers.sh` removes labelled containers whose test process is gone. Before that, `v0.10.27` (`baukit-egress` adds `AddressPolicy::AllowPrivate` with a non-empty list of private networks, so an in-cluster client reaches listed cluster services, plain HTTP included, while metadata addresses and unlisted private ranges stay blocked. `AddressPolicy` and `EgressOptions` are no longer `Copy`. Before that, `v0.10.26` (`baukit-core` adds `ProcessKind::Agent`, so a second product binary such as an in-cluster collector reports `service.name` `<product>-agent` while doctor keeps one product identity. Before that, `v0.10.25` (`@baukit/auth-native`, `@baukit/auth-web` and `@baukit/auth-node` store `issuedAt` with each session and cap the refresh window at half the access token lifetime, so a token shorter than `refreshLeewaySeconds` no longer refreshes on every call; stored sessions without `issuedAt` are discarded. Before that, `v0.10.24` (generated mobile apps pin Expo 57.0.28, expo-auth-session 57.0.15, expo-crypto 57.0.4, expo-router 57.0.26 and jest-expo 57.0.6; `[ports] web_extra = [8082, 19006]` declares other browser clients, and with `web` declared doctor accepts Keycloak origins, redirects and backend `authorized_parties` on `web`, `web_preview`, `metro` and every `web_extra` port, while Vite, the stack Playwright default and the PKCE redirect default still use `web`; the generated Android QA script creates a stock pixel_6 AVD, applies config only from `BAUKIT_QA_AVD_CONFIG`, recreates pixel_7 AVDs and skips `sdkmanager` when every SDK package is installed; `baukit-http` maps any 5xx with a valid `Retry-After` to a capped `RetryAfter`, and `EgressOptions::with_error_body_limit` keeps a bounded `ErrorBody` with its problem code on `EgressError::Status`, keeping the status when the body read fails; `ApiTokenFormat::generate_secret()` returns a `GeneratedApiToken` with its `hash_api_token` hash and display prefix for products with their own token tables; `Principal::from_api_token(subject, token)` is public. Before that, `v0.10.23` (baukit.toml `[ports]` accepts optional host-only `web`, `web_preview` and `metro` entries; generation renders them into the Vite config, Keycloak realm and reconcile origins, backend `authorized_parties`, PKCE login default, Playwright defaults and Expo `--port` scripts; doctor checks those sites, `RCT_METRO_PORT` and Metro `adb reverse` lines, and rejects duplicate effective host ports such as Metro on the default Keycloak 8081; products declaring service `[ports]` without `port_offset` get matching Makefile and Helm port values. Before that, `v0.10.22` (`baukit doctor` accepts `[doctor] waive = [{ path, reason }]` entries for expected files a product covers another way, prints them as notes, and rejects blank reasons and stale, unknown, duplicate, absolute or `..` paths; the generated web e2e geometry helper drops a dead `elementHandle` null check. Before that, `v0.10.21` (API-token principals return the token's `created_at` from `Principal::authenticated_at()`, so tokens minted after Retain re-onboarding pass the erasure fence while pre-erasure tokens stay fenced. Before that, `v0.10.20` (`baukit_core::limits::check_range(measured, minimum, maximum)` and `@baukit/data-contracts` `checkRange` check inclusive whole-number bounds and reject inverted bounds; limits policies accept `ranges` entries with `minimum` and `maximum`, and generated products return `out_of_range` with field, measured value, minimum and maximum; `ErasureService::preflight` checks the key, replay receipt and session fence before `erase_with_suite` revokes suite links, so a replayed key or a fenced token no longer revokes a re-onboarded profile's links; re-erasure needs `UPDATE (erased_at)` on `erasure_fences`; `baukit doctor` workspace discovery skips `fixtures/` and `testdata/` manifests like the pin census. Before that, `v0.10.19` (an erased subject can onboard again with an empty profile once every erasure operation completed and the OIDC `auth_time` is after `erasure_fences.erased_at`, so tokens from before or during erasure stay rejected; `Principal::authenticated_at` carries `auth_time`, and `erasure_fences` gains `erased_at` through `POSTGRES_REONBOARDING_MIGRATION_SQL`; `@baukit/auth-web` and `@baukit/auth-native` accept `clearSession({ reason: 'profile_erased' })`, which emits that session-expired reason and forces the next OIDC login with `prompt=login`; `ScopedPersistenceLifecycle.handleProfileErased()` erases the last local partition; generated products key local data by server profile id; the `baukit-jobs` runner keeps polling the handler and timeout while the cancellation check waits for a pool connection; v0.10.18 made the `baukit-jobs` runner drop the handler future before recording a timeout or cancellation, so a handler transaction holding the job row cannot block the runner; v0.10.17 added `FixedUtcJitter` with `FixedUtcInterval::run_at`, transactional cancellation by idempotency key or literal prefix, and `check_lease_in_transaction`; the generated web e2e helper `expectMinimumInteractiveTargetSize` measures all visible targets in one browser call since v0.10.16; generated backend Dockerfiles take `CARGO_TARGET_CACHE_PREFIX` since v0.10.15; `@baukit/navigation` `SectionPicker` takes `replace` since v0.10.14).

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
| Mobile runtime | Expo SDK | 57.0.32 (RN 0.86.3, React 19.2.3) | React/RN versions follow the Expo SDK, verified with Expo Doctor |
| Mobile navigation | Expo Router | 57.0.29 | Generated mobile template baseline with Expo Router and `Stack.Protected` in the auth overlay; `react-native-screens` 4.26.2, `react-native-safe-area-context` 5.7.0, `react-native-reanimated` 4.5.1, `react-native-worklets` 0.10.1, and `react-native-gesture-handler` 2.32.0. |
| Remote state | TanStack Query | 5 | |
| Web routing | TanStack Router | current v1 | re-verify TanStack Start status separately |
| Local state | Zustand | 5 | |
| Accessibility behavior | `@baukit/a11y-core` | 0.5.2 | Overlay focus, inert, announcements and reduced motion (both also on the `/web` entry). React peer range is `^19.2.0`; React Native is optional, and a plain web app imports `@baukit/a11y-core/web` instead. |
| Localization behavior | `@baukit/localization-core` | 0.5.2 | Locale resolution, catalog key comparison, stable-code localization, timezone-safe civil-date arithmetic, and local-time resolution with required gap and fold policies. |
| Local notification planning | `@baukit/notifications-core` | 0.5.2 | Zoned occurrence resolution through `resolveZonedLocalTime`, horizon filtering, deterministic keep, cancel, and schedule sets, and owner-scoped replacement over a platform port. No dependencies besides `@baukit/localization-core`. |
| Local notification delivery | `@baukit/notifications-expo` / `expo-notifications` | 0.5.2 / 57.0.21 | Optional adapter. Imports only types from `expo-notifications`; the product passes the module in. DATE triggers, per-item results, no cancel-all. |
| Mobile purchases | `@baukit/purchases-core` / `@baukit/purchases-revenuecat` / `react-native-purchases` | 0.14.1 / 0.14.1 / 10.14.0 | Store-neutral flow gated on the backend-verified entitlement. The adapter imports only types from `react-native-purchases`; the app passes the module in. No config plugin. Since 0.14.1 `useFeature` returns `FeatureAccess`, `PurchasesProvider` requires `productId`, and the entitlement comes from the kit's `GET /me/entitlement`; upgrade by passing `productId` and switching on the access status. |
| Platform plan kit | `baukit-suite` `platform` / `@baukit/suite-client` / Schlüsselbund hub | 0.14.1 / 0.14.1 / C27 (`aa3204d`) | `AccountPlan`, `require_feature` and the `/me/entitlement` and `/me/credits` router. The bucketed `CreditBalance` needs the hub at C21 (`f8bc162`) or later and the `billing` ticket claim needs C27; upgrade the hub first, then read `total` and `packs[].available`. |
| Preference behavior | `@baukit/preferences-core` | 0.5.2 | Identity guard and repository store, with `null` repository records treated as missing. |
| Node device authentication | `@baukit/auth-node` | 0.5.2 | Node 24 OIDC device authorization with S256 PKCE, bounded responses and timeouts, refresh rotation, and a locked local profile cache. Plain HTTP requires an explicit loopback-only development policy. The `/keycloak-testing` entry holds Keycloak e2e helpers. |
| Provider registry | `@baukit/integrations-client` | 0.5.2 | Typed product connectors, stable registration order, and immutable connection-state overlays. |
| Client sync primitives | `@baukit/sync-client` | 0.5.2 | Scheduler with optional retry, request-function and HTTP transports, status store, push-batch ranking, a persisted hybrid logical clock, and tombstone-horizon conformance. The optional `@baukit/sync-client/expo` entry uses Expo Network 57.0.2 and React Native 0.86.3; the root entry has no runtime dependencies and no React. |
| PWA cache strategy | `@baukit/pwa-web` | 0.5.2 | ESM and CJS builds, request classification, `navigationFallback`, and strategy execution for a product-owned service worker; no dependencies and no service-worker globals. |
| MCP server | `rmcp` | 3.9.0 | `--mcp --backend --auth oidc` generates Rust Streamable HTTP with resource OAuth and service ports. |
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
| Expo SDK | 57.0.31 | SDK 57 is the latest stable SDK. Its native package versions remain together. |
| React Native | 0.87.1 | Expo SDK 57 uses 0.86.3, including the Babel and Jest presets. |
| React on mobile | 19.3.0 | Expo SDK 57 uses 19.2.3. Web uses 19.3.0. |
| Jest and @jest/globals | 30.5.2 | jest-expo 57 and the React Native Jest preset use Jest 29. Types use @types/jest 29.5.14. |
| Babel | 8.0.6 | The React Native Babel preset uses Babel 7 plugins. |
| test-renderer | 1.3.0 | Its React reconciler requires React 19.3. The Expo set uses React 19.2. |
| AsyncStorage | 3.1.1 | The [Expo SDK 57 bundle](https://github.com/expo/expo/blob/sdk-57/packages/expo/bundledNativeModules.json) uses 2.2.0 for Expo Go. Version 3 uses a new native module. |
| Gesture Handler | 3.3.0 | Expo SDK 57 uses 2.32.0 with its navigation packages. |
| Reanimated / Worklets | 4.7.1 / 0.13.0 | Expo SDK 57 uses 4.5.1 / 0.10.1. Screens 4.26.2 and Safe Area Context 5.7.0 stay on the same SDK baseline. |
| Gradle / AGP / Kotlin / NDK | 9.8.2 / 9.5.0 / 2.4.21 / r29 | Baukit pins none of them. The Expo SDK 57 template generates Gradle 9.3.1, and React Native 0.86.3 pins AGP 8.12.0, Kotlin 2.1.20, and NDK 27.1.12297006. Overriding them would go past what Expo SDK 57 supports. |
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
