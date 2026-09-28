# @baukit/integrations-client

## 0.5.0

### Minor Changes

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

- 1391492: Let provider registries retain typed product connectors and apply immutable connection-state
  overlays while preserving registration order.
- Release the coordinated baukit 0.2.1 train.

## 0.2.0

### Minor Changes

- First public release. It includes a connection-health reducer, OAuth session
  coordinator, and provider registry.
