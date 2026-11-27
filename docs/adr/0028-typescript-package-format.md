# ADR 0028: TypeScript packages ship ES modules, pass a packed-export test, and keep vendor SDKs in adapter packages

## Status

Accepted, 2026-11-27.

## Context

Products consume `@baukit/*` from Vite web apps, Expo apps, Node backends,
Vitest and Jest. Each resolver reads `exports` conditions differently. A
package that builds in the workspace can still fail after `pnpm pack`, because
the archive misses a file or an export condition points nowhere. Jest resolves
with CommonJS conditions, so an export with only `import` fails there.

Vendor SDKs such as PostHog, RevenueCat, Dexie, Expo SQLite and
expo-notifications are heavy and change often. A product that wants the shared
behaviour should not have to install a vendor it does not use.

## Decision

Every package under `typescript/packages/` sets `"type": "module"` and builds
ES modules into `dist/`. Each export lists `import` and a matching `default`
condition, so Node 24 and Jest load it through `require(esm)`. `./vitest` test
subpaths list only `import`, because Vitest is ESM-only.

Two packages also ship a CommonJS build under `dist/cjs/` with a `require`
condition:

- `@baukit/pwa-web` exports `.` as CommonJS too, so existing
  `require('@baukit/pwa-web')` callers keep working.
- `@baukit/suite-client` exports `./peers` and `./entitlements` as CommonJS, for
  Expo config plugins such as `plugins/with-suite-link-queries.cjs`. Its `.`
  export is ESM only.

Shared behaviour lives in a core package with no vendor SDK dependency, and
each vendor or storage engine gets its own adapter package:

| Core | Adapters |
|---|---|
| `@baukit/analytics-core` | `@baukit/analytics-posthog-web`, `@baukit/analytics-posthog-native` |
| `@baukit/data-contracts` | `@baukit/data-contracts-dexie`, `@baukit/data-contracts-expo-sqlite` |
| `@baukit/notifications-core` | `@baukit/notifications-expo` |
| `@baukit/purchases-core` | `@baukit/purchases-revenuecat` |

Core packages may still take React or React Native as peers when they ship
hooks or components, as `@baukit/a11y-core` does.

## Enforced by

- `typescript/scripts/test-packed-exports.mjs`. Each package's `test` script
  runs it after Vitest. It packs the package, installs the archive in a scratch
  directory and resolves every export with CommonJS `require` conditions. It
  fails when an export resolves outside the package or to a file missing from
  the archive. `--esm-only <subpath>` asserts that a subpath does not resolve
  under `require`. `--load <subpath>` also loads it. It also checks packed
  source maps. The `ts-check` gate step runs every package's `test`.
- `@baukit/pwa-web` and `@baukit/eslint-config` do not run the packed-export
  test. Adding it to their `test` scripts would close that gap.
- The `package-dependencies` step runs `scripts/check-package-dependencies.py`,
  which rejects `workspace:`, `link:` and `file:` specifiers in packed archives.
- Nothing checks `"type": "module"` or the core and adapter split. A check that
  reads every `package.json` and rejects a `type` other than `module`, or a
  vendor SDK in a core package's `dependencies`, would.

## Consequences

Products get packages that resolve the same way in bundlers, Node and Jest,
and install only the vendor adapters they use. Jest still needs a transform
for ESM. The two CommonJS builds double those packages' build output and keep a
second `tsconfig.cjs.json` each.

## History

- 2026-08-25, `af6e2b4f`: baukit 0.1.0 shipped `@baukit/pwa-web` with its
  CommonJS `require` export.
- 2026-09-27, `8d268e17`: packages published `default` export conditions and
  gained the packed-export test, so Jest resolves `@baukit/*` without a module
  map.
- 2026-10-07, `a0619b1d`: `@baukit/suite-client` added CommonJS `./peers` for
  Expo config plugins.
