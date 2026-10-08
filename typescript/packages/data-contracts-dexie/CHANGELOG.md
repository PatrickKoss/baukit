# @baukit/data-contracts-dexie

## Unreleased

## 0.10.4

### Patch Changes

- Release the coordinated baukit 0.10.4 train.
- Updated dependencies
  - @baukit/data-contracts@0.10.4

## 0.10.3

### Patch Changes

- Release the coordinated baukit 0.10.3 train.
- Updated dependencies
  - @baukit/data-contracts@0.10.3

## 0.10.2

### Patch Changes

- Release the coordinated baukit 0.10.2 train.
- Updated dependencies
  - @baukit/data-contracts@0.10.2

## 0.10.1

### Patch Changes

- Release the coordinated baukit 0.10.1 train.
- Updated dependencies
  - @baukit/data-contracts@0.10.1

## 0.10.0

- Move shipped notes out of Unreleased into their release sections.

### Minor Changes

- Release the coordinated baukit 0.10.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/data-contracts@0.10.0

## 0.9.0

### Minor Changes

- Release the coordinated baukit 0.9.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/data-contracts@0.9.0

## 0.8.0

### Minor Changes

- Release the coordinated baukit 0.8.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/data-contracts@0.8.0

## 0.7.4

### Patch Changes

- Release the coordinated baukit 0.7.4 train.
- Updated dependencies
  - @baukit/data-contracts@0.7.4

## 0.7.3

- Publish the TypeScript sources referenced by JavaScript and declaration maps. Check source paths in the packed archive.

### Patch Changes

- Release the coordinated baukit 0.7.3 train.
- Updated dependencies
  - @baukit/data-contracts@0.7.3

## 0.7.2

### Patch Changes

- Release the coordinated baukit 0.7.2 train.
- Updated dependencies
  - @baukit/data-contracts@0.7.2

## 0.7.1

### Patch Changes

- Release the coordinated baukit 0.7.1 train.
- Updated dependencies
  - @baukit/data-contracts@0.7.1

## 0.7.0

### Minor Changes

- Release the coordinated baukit 0.7.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/data-contracts@0.7.0

## 0.6.0

### Minor Changes

- be675db: Raised peer floors to the versions Baukit now tests against. `@baukit/ui-tokens` takes ESLint 10 (`eslint ^10.11.0`), so products can leave ESLint 9. The Expo SDK 57 peers are `react-native ^0.86.3`, `expo-auth-session ^57.0.13`, `expo-secure-store ^57.0.4`, `expo-web-browser ^57.0.3`, `expo-sqlite ^57.0.3`, `expo-notifications ^57.0.21`, and `expo-network ^57.0.2`. `@baukit/data-contracts-dexie` needs `dexie ^4.4.6`.
- Release the coordinated baukit 0.6.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/data-contracts@0.6.0

## 0.5.2

### Patch Changes

- Release the coordinated baukit 0.5.2 train.
- Updated dependencies
  - @baukit/data-contracts@0.5.2

## 0.5.1

### Patch Changes

- e26045b: Add `KeyValueStore.clearPrefix(prefix)`, which deletes every key that starts with `prefix`. Matching is exact and case-sensitive, no character is a wildcard, and an empty prefix clears the store. `InMemoryKeyValueStore`, `DexieKeyValueStore` (one IndexedDB key range), and the Expo SQLite key-value store (a UTF-8 byte prefix match inside the store's namespace) implement it, and `describeKeyValueContract` checks it, including SQL `LIKE` wildcards, case, emoji, and U+FFFF. The Dexie real-browser suite now runs the key-value contract too.

  Breaking: `KeyValueStore` gains a required method, so a product's own `KeyValueStore` implementation must add `clearPrefix`.

- Release the coordinated baukit 0.5.1 train.
- Updated dependencies [06cc669]
- Updated dependencies [d423b98]
- Updated dependencies [e26045b]
- Updated dependencies
- Updated dependencies [403805a]
  - @baukit/data-contracts@0.5.1

## 0.5.0

### Minor Changes

- 8d268e1: Add a `default` export condition next to `import` on every export except the ESM-only `./vitest` subpaths. Jest and other CommonJS-condition resolvers now find `@baukit/*` without a `moduleNameMapper`. Each package's `test` script packs the package and resolves every export under `require` conditions from the archive.
- Release the coordinated baukit 0.5.0 train.

### Patch Changes

- Updated dependencies [8d268e1]
- Updated dependencies [acaab1b]
- Updated dependencies
- Updated dependencies [d9225d5]
- Updated dependencies [a9fa22e]
  - @baukit/data-contracts@0.5.0

## 0.4.0

### Minor Changes

- Release the coordinated baukit 0.4.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/data-contracts@0.4.0

## 0.3.0

### Minor Changes

- Release the coordinated baukit 0.3.0 train.

### Patch Changes

- Updated dependencies [40882f6]
- Updated dependencies
- Updated dependencies [5472d3d]
  - @baukit/data-contracts@0.3.0

## 0.2.1

### Patch Changes

- Release the coordinated baukit 0.2.1 train.
- Updated dependencies
  - @baukit/data-contracts@0.2.1

## 0.2.0

### Minor Changes

- Release the coordinated baukit 0.2.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/data-contracts@0.2.0

## 0.1.2

### Patch Changes

- Release the coordinated baukit 0.1.2 train.
- Updated dependencies
  - @baukit/data-contracts@0.1.2

## 0.1.1

### Patch Changes

- Release the coordinated baukit 0.1.1 train.
- Updated dependencies
  - @baukit/data-contracts@0.1.1

## 0.1.0

### Minor Changes

- First public release of `@baukit/data-contracts-dexie`.
