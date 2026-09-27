---
'@baukit/a11y-core': minor
'@baukit/analytics-core': minor
'@baukit/analytics-posthog-native': minor
'@baukit/analytics-posthog-web': minor
'@baukit/api-runtime': minor
'@baukit/auth-native': minor
'@baukit/auth-node': minor
'@baukit/auth-web': minor
'@baukit/data-contracts': minor
'@baukit/data-contracts-dexie': minor
'@baukit/data-contracts-expo-sqlite': minor
'@baukit/events': minor
'@baukit/integrations-client': minor
'@baukit/localization-core': minor
'@baukit/preferences-core': minor
'@baukit/sync-client': minor
'@baukit/ui-tokens': minor
---

Add a `default` export condition next to `import` on every export except the ESM-only `./vitest` subpaths. Jest and other CommonJS-condition resolvers now find `@baukit/*` without a `moduleNameMapper`. Each package's `test` script packs the package and resolves every export under `require` conditions from the archive.
