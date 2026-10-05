# @baukit/analytics-core

## Unreleased

- Publish the TypeScript sources referenced by JavaScript and declaration maps. Check source paths in the packed archive.

## 0.7.2

### Patch Changes

- Release the coordinated baukit 0.7.2 train.

## 0.7.1

### Patch Changes

- Release the coordinated baukit 0.7.1 train.

## 0.7.0

### Minor Changes

- Release the coordinated baukit 0.7.0 train.

## 0.6.0

### Minor Changes

- Release the coordinated baukit 0.6.0 train.

## 0.5.2

### Patch Changes

- Release the coordinated baukit 0.5.2 train.

## 0.5.1

### Patch Changes

- Release the coordinated baukit 0.5.1 train.

## 0.5.0

### Minor Changes

- 4f4a873: Add exact-key scrubbing and `scrubErrorEvent` for crash reports. `scrubProperties` and `AnalyticsClient` accept `exactBlockedKeys`, which redact a key only when the normalized key is equal. `scrubErrorEvent` redacts request headers, bodies, query strings, breadcrumb data, and frame variables, and keeps event and trace IDs and stack frame locations. Break: `ip`, `ip_address`, `remote_addr`, `x_forwarded_for`, and `x_real_ip` are now redacted by default through `DEFAULT_EXACT_BLOCKED_KEYS`.
- bc05828: Add `HydratedAnalyticsStorage` to the new `@baukit/analytics-posthog-native/storage` subpath. It loads a list of keys from an asynchronous store such as AsyncStorage once, then serves the synchronous `AnalyticsStorage` port from memory and writes those keys through. A key that fails to read starts absent without dropping the others. The subpath does not import `posthog-react-native`.

  Add `analyticsStorageKeys(prefix)` to `@baukit/analytics-core`. It returns the consent, anonymous ID, user ID, and alias guard keys that `AnalyticsClient` uses.

  `posthog-react-native` is now an optional peer of `@baukit/analytics-posthog-native`. Apps that use the transport must install it themselves, which the setup instructions already said.

  The mobile template now calls `HydratedAnalyticsStorage` instead of generating its own class. Products that copied the template class can delete it.

- 8d268e1: Add a `default` export condition next to `import` on every export except the ESM-only `./vitest` subpaths. Jest and other CommonJS-condition resolvers now find `@baukit/*` without a `moduleNameMapper`. Each package's `test` script packs the package and resolves every export under `require` conditions from the archive.
- Release the coordinated baukit 0.5.0 train.

## 0.4.0

### Minor Changes

- Release the coordinated baukit 0.4.0 train.

## 0.3.0

### Minor Changes

- Release the coordinated baukit 0.3.0 train.

## 0.2.1

### Patch Changes

- Release the coordinated baukit 0.2.1 train.

## 0.2.0

### Minor Changes

- Release the coordinated baukit 0.2.0 train.

## 0.1.2

### Patch Changes

- Release the coordinated baukit 0.1.2 train.

## 0.1.1

### Patch Changes

- Release the coordinated baukit 0.1.1 train.

## 0.1.0

### Minor Changes

- First public release of `@baukit/analytics-core`.
