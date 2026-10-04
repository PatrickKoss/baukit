# @baukit/analytics-posthog-native

## [Unreleased]

- Verify consent withdrawal and storage against posthog-react-native 4.78.4.

## 0.7.2

### Patch Changes

- Release the coordinated baukit 0.7.2 train.
- Updated dependencies
  - @baukit/analytics-core@0.7.2

## 0.7.1

### Patch Changes

- Release the coordinated baukit 0.7.1 train.
- Updated dependencies
  - @baukit/analytics-core@0.7.1

## 0.7.0

### Minor Changes

- Release the coordinated baukit 0.7.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/analytics-core@0.7.0

## 0.6.0

### Minor Changes

- be675db: The consent purge now also clears `ai_capture_queue`, the persisted queue that `posthog-react-native` 4.78 (through `@posthog/core` 1.55) added. Before this, events queued there survived a consent withdrawal. `PostHogNativeClient.setPersistedProperty` takes the key as the string union `` `${PostHogPersistedProperty}` ``, so fakes can pass plain strings. The `posthog-react-native` peer floor is now `^4.78.3`.
- Release the coordinated baukit 0.6.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/analytics-core@0.6.0

## 0.5.2

### Patch Changes

- Release the coordinated baukit 0.5.2 train.
- Updated dependencies
  - @baukit/analytics-core@0.5.2

## 0.5.1

### Patch Changes

- Release the coordinated baukit 0.5.1 train.
- Updated dependencies
  - @baukit/analytics-core@0.5.1

## 0.5.0

### Minor Changes

- bc05828: Add `HydratedAnalyticsStorage` to the new `@baukit/analytics-posthog-native/storage` subpath. It loads a list of keys from an asynchronous store such as AsyncStorage once, then serves the synchronous `AnalyticsStorage` port from memory and writes those keys through. A key that fails to read starts absent without dropping the others. The subpath does not import `posthog-react-native`.

  Add `analyticsStorageKeys(prefix)` to `@baukit/analytics-core`. It returns the consent, anonymous ID, user ID, and alias guard keys that `AnalyticsClient` uses.

  `posthog-react-native` is now an optional peer of `@baukit/analytics-posthog-native`. Apps that use the transport must install it themselves, which the setup instructions already said.

  The mobile template now calls `HydratedAnalyticsStorage` instead of generating its own class. Products that copied the template class can delete it.

- 8d268e1: Add a `default` export condition next to `import` on every export except the ESM-only `./vitest` subpaths. Jest and other CommonJS-condition resolvers now find `@baukit/*` without a `moduleNameMapper`. Each package's `test` script packs the package and resolves every export under `require` conditions from the archive.
- Release the coordinated baukit 0.5.0 train.

### Patch Changes

- Updated dependencies [4f4a873]
- Updated dependencies [bc05828]
- Updated dependencies [8d268e1]
- Updated dependencies
  - @baukit/analytics-core@0.5.0

## 0.4.0

### Minor Changes

- Release the coordinated baukit 0.4.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/analytics-core@0.4.0

## 0.3.0

### Minor Changes

- Release the coordinated baukit 0.3.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/analytics-core@0.3.0

## 0.2.1

### Patch Changes

- Release the coordinated baukit 0.2.1 train.
- Updated dependencies
  - @baukit/analytics-core@0.2.1

## 0.2.0

### Minor Changes

- Release the coordinated baukit 0.2.0 train.

### Patch Changes

- Updated dependencies
  - @baukit/analytics-core@0.2.0

## 0.1.2

### Patch Changes

- Release the coordinated baukit 0.1.2 train.
- Updated dependencies
  - @baukit/analytics-core@0.1.2

## 0.1.1

### Patch Changes

- Release the coordinated baukit 0.1.1 train.
- Updated dependencies
  - @baukit/analytics-core@0.1.1

## 0.1.0

### Minor Changes

- First public release of `@baukit/analytics-posthog-native`.
